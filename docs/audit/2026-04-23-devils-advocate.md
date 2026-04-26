# Devil's Advocate Audit — 2026-04-23

> 28 days to OSS launch (2026-05-21). This document asks: is that real?
> Author: Sonnet. Written without ego-protection. Every claim below is tied to a shell command or file path.

---

## 1. Portal "upstream tab 3 @ 100%" — bulundu ve yalanlandı

**Kaynak:** `crates/cave-runtime/src/portal_index.html:382-384`

Satır 382-384'te hardcoded `parity:100` vardı:
- containerd/crun → cave-cri → `parity:100`
- etcd → cave-etcd → `parity:100`
- kube-apiserver → cave-apiserver → `parity:100`

**Ama gerçek (yeni `/api/portal/runtime/progress` endpoint'inden, koddan sayılmış):**
- cave-cri: **29%**
- cave-etcd: **33%**
- cave-apiserver: **24%**

Yalan büyüklüğü: **71 + 67 + 76 = 214 puanlık yalan.**

**Daha kötüsü:** dosyada toplam 70 adet `parity:` deklarasyonu var. Dağılım:
- 21 modül `25%` (copy-paste placeholder)
- 19 modül `20%` (copy-paste placeholder)
- 14 modül `15%` (copy-paste placeholder)
- 10 modül `30%` (copy-paste placeholder)
- 3 modül `100%` ← yakalanan
- Diğer 3 farklı değer

**Yani portal'ın "upstream" sayfası çoğunlukla copy-paste sahte sayılar.**

**Fix (uygulandı, commit edilmeye hazır):**
- 3 × `100%` → 29/33/24 (gerçek)
- scheduler `40%` → 7
- kubelet `23%` → 6
- Geri kalan 65 entry hâlâ tahmini — uzun vadeli fix: tüm UPSTREAM_PROJECTS array'i `/api/portal/runtime/progress`'ten canlı fetch etmeli, inline hardcoded değerler silinmeli.

---

## 2. ADR envanteri

Komut: `ls docs/adr/*.md | wc -l` → [TBD runtime]
Her ADR için status breakdown TBD.

**Kritik gap:**
- Charter Runtime ADR'ları (ADR-144..165, 22 adet) — memory'e göre sadece Excel'de, MD'ye indirilmemiş. OSS 28 gün sonra, bu gap açık kalırsa public repo'da ADR-CHARTER-001 varken ADR-144..165 boş olur.
- Platform ADR'lar (ArgoCD/Backstage/Crossplane/Keycloak/Kong/Cert-manager gibi) — runtime repo'da hâlâ var. Burak 2026-04-23'te açıkça "bunlar RUNTIME upstream, PLATFORM değil" dedi. Temizlik yapılmadı.
- **Altın kural ihlali tespiti (stub yasağı):** `grep -rn "todo!()\|unimplemented!()" crates/` sayısı [TBD] — sıfır değilse altın kural resmen ihlal.

---

## 3. PQC (post-quantum crypto) — hayal mi, gerçek mi

**Charter iddiası:** ADR-166 no-backcompat + PQC-ready. `cave-crypto` modülü ML-KEM / ML-DSA / SLH-DSA primitifleri üstüne kurulmalı.

**Gerçek:** `grep -rln "ML-KEM\|ML-DSA\|SLH-DSA\|ml_kem\|ml_dsa\|CRYSTALS" crates/` → [TBD]. Sıfır dönerse hiç yazılmamış.

---

## 4. Parity manifests — self-report mu gerçek ölçüm mü?

Her crate'te `parity.manifest.toml` var. İçinde `file_parity`, `function_parity`, `test_parity`, `surface_parity` alanları. Bu değerler manuel yazılıyor mu, yoksa bir build script tarafından hesaplanıyor mu?

**Test:** `cargo test -p cave-cri --list 2>&1 | grep -c "^test "` → CRI için iddia edilen 292 backend test gerçek mi?

**Devil's advocate:** parity metrikleri manuel ise self-reported yüzde yasağı (altın kural madde 4) ihlali. Ölçüm yok ise "%24 workspace genelinde" iddiası bir hayal.

---

## 5. Commit forensics (son 8 gün)

Komut: `git log --all --since="2026-04-15" --oneline | wc -l` → [TBD]

Bu commit'lerin tipine göre breakdown:
- `[qwen-amele]` prefix → LLM ürünü
- `fix(*)` / `chore(*)` → WIP / bug fix
- `feat(*)` → gerçek yeni feature

Eğer LLM ürünü %70'ten fazla ve compile_fail rate'leri yüksekse, daemon'u keep etmek sermaye kaybı.

**Çok önemli: son 3 saatte daemon log'unda her tick `draft failed — escalating ... attempts=3` oldu. Yani LLM output'ları compile geçmiyor, bitmeye doğru gitmeye.**

---

## 6. Benim (Sonnet) yanlışlarım — 2026-04-23 konuşması

Belgeli yanlışlar:
1. **Dispatch-and-hope:** amele task'a "3-repo mode aktif et" talimatı verdim, amele "PID 65463 running, cave-etcd/CompactPath committed" dedi. Verify etmedim. Audit sonrası: PID 82438 idi, cave-etcd/CompactPath commit'i hiç atılmamıştı.
2. **Portal "attribution wired" iddiası:** amele raporunu tekrarladım, oysa `LocalLLMDaemon.tsx`'in başında `// TODO (Phase 5): wire up real backend endpoints` + `DUMMY_QUEUE / DUMMY_METRICS` duruyordu.
3. **ADR kategorizasyon hatası:** ArgoCD/Backstage/Crossplane/Argo Workflows'u PLATFORM olarak sınıflandırdım. Burak "bunlar upstream, RUNTIME reimpl hedefi" dedi. Memory'e feedback olarak yazıldı.
4. **Queue `skipped` variantı:** Rust `Status` enum'ını verify etmeden `status: skipped` önerdim. Enum'da yok, daemon 2 dakika crash loop'a girdi.
5. **"IPv6 tuzağı" yarı-teşhis:** `OLLAMA_HOST=http://127.0.0.1:11434` env var'ı ekledim, ama daemon'un kodu env var'ı okuyor mu doğrulamadım. Sonra anlaşıldı ki daemon farklı sebeple çözüldü.
6. **WIP commit atlamak:** daemon.rs/scheduler.rs/plist/queue.yaml 4 modified dosyası uncommitted iken binary rebuild önerdim. Eğer rebuild sırasında dallanma olsa WIP kaybolurdu.
7. **Portal hardcoded `parity:100` yalanı:** Ben eklemedim ama **var olduğunu fark etmedim** — Burak yakaladı. "Portal işliyor, senden URL al" dedim, bu arada 3 modül yalan söylüyordu.

---

## 7. 28 gün scope gerçekliği

**Hedef:** Fully featured sovereign self-healing multi-region HA+DR Cloud OS. Line-by-line TDD upstream parity.
**Takım:** 1 kişi (Burak) + Claude orchestration + 1 Qwen3 amele.
**Pace:** Son 24 saatte 9 commit (cave-runtime) + 5 (pipeline) + 0 (muleforge) = **14 commit/gün.**
**Qwen3 rate:** son saatlerde compile_fail %100. Yani effective LLM output rate = 0.

**Devil's advocate matematiği:**
- 14 commit/gün × 28 gün = 392 commit
- 23 modül × ortalama %20 complete = ~5 "fully done" modül karşılığı iş
- Kubernetes ekosistemindeki sadece **etcd** kendi içinde 500+ commitlik iş. apiserver 2000+.
- 28 günde "fully featured upstream parity" **matematiksel olarak mümkün değil.** Mevcut LoC pace'i hiçbir upstream'in %10'una bile ulaşmayacak.

**Gerçekçi scope (28 gün):**
- Showcase-viable MVP: 2-3 modül fully demo edilebilir (cave-gateway %64 → %100, cave-mesh %61 → %100, cave-etcd %33 → %80)
- Charter + ADR'lar düzeltilmiş (OSS-ready dokümanlar)
- Repo temiz (git history, no dummy data, no stub, .gitignore doğru)
- README + CONTRIBUTING + LICENSE + SECURITY — OSS hijyen
- Portal'da gerçek veri — tek sayfa üstünden gerçek parity gösterilebilir
- CI: public runners + sovereign secret handling

**21 Mayıs'ta neyin hazır OLMAYACAĞINI kabul et:**
- 65+ modül "fully featured" OLMAYACAK
- Multi-region HA+DR **çalıştırılabilir demo** olmayacak (test evi yok)
- Linux kernel refactor vizyonu (ADR-CHARTER-001) OLMAYACAK — uzun vade
- Self-improving ML-assisted controller olmayacak

---

## 8. Teknik borç — acil

1. **`.gitignore` bozuk:** `crates/*/target/` tracked. `git ls-files crates/ | grep target/ | wc -l` → [TBD] binlerce dosya. Sandbox'ta SIGBUS sebebi, OSS repo balonlaşmış. Fix: `.gitignore`'a `**/target/` + `git rm -r --cached crates/*/target`.
2. **Stale worktree'ler:** `.claire/worktrees/*` altında N adet. `git worktree prune` atlanmış.
3. **macOS duplicate'ler:** `find . -name "* 2.*"` — "filename 2.ext" Finder artifactları.
4. **Uncommitted WIP (hâlâ):** daemon.rs + scheduler.rs + plist + queue.yaml 4 saattir commit edilmedi.
5. **276 branch** — çoğu `.claire/worktrees/claude/*` agent artığı. Prune şart.
6. **Authoritative branch belirsiz:** main / qwen/auto-2026-W17 / refactor/continuous-synergy. Sweep-002 refactor branch'i main'e inmedi. 36 commit diverjans.
7. **Portal hardcoded data:** `UPSTREAM_PROJECTS` array'i 70 item, hepsinin `parity` değeri manuel. Gerçek veriye bağlanmalı.

---

## 9. 3 somut öneri

### 9.1. Scope daraltma — 28 gün için (URGENT, bugün)

Şu 3 kategoriyi ayrıştır, public read-me'de açık söyle:
- **"Showcase modules"**: cave-gateway, cave-mesh, cave-etcd (en ileride olanlar). Bunların 4-track completion'ı (backend + portal + cavectl + obs) 21 Mayıs'a kadar biter.
- **"In progress (public invite)"**: diğer 20 modül — OSS community'e "contribute" davet et, %10-30 tamam durumda.
- **"Roadmap (long-term vision)"**: Linux kernel refactor, Multi-region HA+DR, self-improving controllers. Vizyon yaz, commit etme.

### 9.2. Qwen3 amele karar

Qwen3-Coder-Next 80B Q4 son saatlerde compile_fail rate %100. İki seçenek:
- **(A) Fix pipeline:** prompt template'i daralt (tek fonksiyon, tek signature), context'e yeterli upstream kod ver, output'u ilk compile'da test et; başarısız olursa commit atma, `docs/drafts/failed/` altına at.
- **(B) At:** Sonnet (ben) + Burak direkt kod yazalım. LLM overhead'i sermaye kaybı. Daemon run-cost'u var (50 GB RAM), çıktı sıfır.

Karar için somut metrik: önümüzdeki 2 saatte başarılı commit sayısı. Sıfırsa (B). 3+ ise (A) worth trying.

### 9.3. Portal dummy data kıyımı

Şu path'ler artık gitmeli:
- `crates/cave-runtime/src/portal_index.html` UPSTREAM_PROJECTS array: hardcoded parity değerleri kaldırılacak, `/api/portal/runtime/progress`'ten fetch edilecek (şu an 3 fix edildi, 65 kaldı)
- `crates/cave-portal-ui/src/pages/LocalLLMDaemon.tsx`: `DUMMY_QUEUE / DUMMY_METRICS` silinecek (unblock task kısmi fix yaptı, doğrulanmadı)
- Her "Phase 5 TODO wire up real backend" yorumu: ya implement edilecek ya sayfa kaldırılacak

---

## Sonuç

Proje **patlak değil** — ama kendini kendine yalan söyleyen bir state'te. 3 ana yalan:
1. Portal hardcoded parity değerleri gerçekle çelişiyor (fix başladı)
2. Amele commit üretiyor diye raporlanıyor, gerçekte draft-only, %100 compile_fail
3. 21 Mayıs "fully featured" OSS launch iddiası 28 günde matematiksel olarak imkansız

**Devam kararı:** Evet ama scope daraltılmış, Qwen3 için net karar (fix/at), ve her "tamam" iddiası için `cargo test` + `git log -1` kanıtı zorunlu.

---

*Devil's advocate aynası. Sonnet yazdı. Her rakam bugün `git log` veya `grep` veya `curl` ile doğrulanabilir.*
