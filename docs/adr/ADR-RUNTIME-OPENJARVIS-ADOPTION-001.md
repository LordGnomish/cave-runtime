# ADR-RUNTIME-OPENJARVIS-ADOPTION-001 — OpenJarvis Adoption (Local-first Personal AI)

**Title:** OpenJarvis Adoption — Local-first Personal AI in cave-agent
**Status:** Accepted
**Scope:** Cave Runtime AI/agent layer
**Category:** AI/LLM, Agent Orchestration
**Decided:** 2026-05-28 (Burak Tartan)
**Related ADRs:** ADR-150 (Hermes Agent Adoption, enterprise), ADR-153 (LLM Gateway MVP), ADR-152 (LLM Tracker), ADR-RUNTIME-DEV-MODE-001 (dev mode runtime), ADR-HOME-OPENJARVIS-ADOPTION-001 (cave-home counterpart — *ayrı repo, ayrı ray*)

---

## Context

**OpenJarvis** (Stanford SAIL / Hazy Research, Apache-2.0) local-first bir agent framework'üdür. On-device, composable agent pattern'leri ve değerlendirme araçları sunar; bir LLM backend'ini değil, backend'leri **orchestrate** eder.

Cave tarafında bizim zaten port ettiğimiz / sardığımız backend'ler var:
- `cave-local-llm` — Ollama + vLLM engine internals (in-process pure-Rust).
- `cave-mlx` — Apple Silicon array core.
- (gelecekte SGLang / llama.cpp muadili yollar.)

OpenJarvis tam olarak bu backend'leri orkestre eden katman: hangi modeli, hangi cihazda, hangi enerji/gecikme/maliyet bütçesiyle çalıştıracağına karar veren composable on-device pattern'ler.

**Konumlandırma — Hermes vs OpenJarvis (complement, rekabet değil):**
- **Hermes (ADR-150)** — *enterprise* agent pozisyonu; worktree-pump pipeline, sunucu-tarafı orkestrasyon.
- **OpenJarvis** — *personal, local-first*; developer workstation'ında, kişisel cihazda çalışan agent.

İkisi birlikte Cave'in **full agent stack**'ini oluşturur: enterprise (Hermes) + personal local-first (OpenJarvis). Dev mode runtime (ADR-RUNTIME-DEV-MODE-001) OpenJarvis'in üzerinde koştuğu yerel runtime'dır.

## Decision

OpenJarvis primitive'leri **mevcut bir crate'e entegre edilir — yeni crate açılmaz.**

Entegre edilen primitive'ler:
1. **Composable on-device patterns** — agent compose/chain/route pattern'leri, local-first çalışma modeli.
2. **Evaluation tools** — energy / latency / cost / accuracy ölçüm ve karşılaştırma araçları. On-device backend seçimini bu metriklerle yönlendirir.
3. **Backend orchestration glue** — `cave-local-llm` (Ollama/vLLM), `cave-mlx` ve gelecekteki backend'leri tek bir orkestrasyon arayüzü altında toplayan yapıştırıcı katman.

Apache-2.0 Python kaynağı → Rust port edilir (Cave golden rule: TDD line-by-line upstream parity, port effort üstlenilir).

> **Hedef crate notu (honest).** Dispatch "cave-agent crate'i" diyor ve "yeni crate yok" diyor. Mevcut workspace'te `cave-agent` adında bir crate **yok**; bugünkü tek agent crate'i `cave-hermes` (`crates/ai/cave-hermes`, ADR-150). "Yeni crate yok" kısıtı iki şekilde karşılanabilir: (a) OpenJarvis primitive'leri `cave-hermes` içine local-first bir alt-modül olarak girer, ya da (b) personal/enterprise ayrımı net tutulmak isteniyorsa `cave-agent` adıyla *yeni* bir crate açmak gerekir ki bu "yeni crate yok" ile çelişir. **Karar implementasyon ray'ine bırakıldı**; bu ADR primitive setini ve konumlandırmayı bağlar, fiziksel crate yerleşimini Phase 1 implementasyon dispatch'i netleştirir. Bu doküman cave-agent ismini dispatch'teki vizyon ismi olarak korur; somut yerleşim cave-hermes alt-modülü en olası sonuçtur.

## Consequences

### Positive
- **Local-first AI katmanı** — kişisel, on-device, sovereign agent yeteneği.
- **Hermes + Jarvis full agent stack** — enterprise + personal eksenleri birlikte kapanır.
- **cave-home natural fit** — kişisel/ev senaryosu (ADR-HOME-OPENJARVIS-ADOPTION-001 counterpart) doğrudan oturur.
- **Backend yatırımı değerlenir** — cave-local-llm / cave-mlx üzerine bir orkestrasyon değeri biner.

### Negative
- **Apache-2.0 Python → Rust port effort** — port maliyeti gerçek; evaluation harness + pattern kütüphanesi hatırı sayılır iş.
- **Hermes ile sınır netliği** — enterprise vs personal sınırı kod seviyesinde net tutulmazsa iki agent yolu karışabilir (yukarıdaki hedef crate notu bu riskin merkezi).

## Cave-home counterpart

> Bu dispatch **cave-runtime** repo'sunadır. cave-home tarafındaki `ADR-HOME-OPENJARVIS-ADOPTION-001` **bu repo'da oluşturulmaz** — cave-home repo'suna **ayrı bir ray** ile yazılır. Aşağıdaki özet yalnızca cross-reference içindir.

**ADR-HOME-OPENJARVIS-ADOPTION-001 (özet, cave-home repo'sunda):**
- **Konum:** cave-home'un kişisel/ev AI deneyimi; OpenJarvis'i ev-senaryosu (kişisel asistan, ev otomasyonu, özel veri üzerinde local-first reasoning) için benimser.
- **İlişki:** cave-runtime tarafı (bu ADR) backend orchestration + evaluation + composable pattern primitive'lerini sağlar; cave-home tarafı bu primitive'leri ev/kişisel ürün deneyimine bağlar.
- **Sınır:** Enterprise orkestrasyon (Hermes) cave-home kapsamı dışıdır; cave-home yalnızca personal local-first yolu kullanır.
- **Eylem:** cave-home repo'sunda ayrı bir ADR ray'i ile yazılacak; bu bölüm yalnızca o ADR'in cave-runtime tarafındaki bağını kayda geçirir.

## Related
- ADR-150 — Hermes Agent Adoption (enterprise pozisyonu, complement)
- ADR-153 — LLM Gateway MVP (LiteLLM yolu; gateway ≠ engine orchestration)
- ADR-152 — LLM Tracker (always-latest model takibi)
- ADR-RUNTIME-DEV-MODE-001 — dev mode runtime (OpenJarvis'in koştuğu yerel runtime)
- ADR-HOME-OPENJARVIS-ADOPTION-001 — cave-home counterpart (ayrı repo, ayrı ray)
- `crates/ai/cave-local-llm` — Ollama/vLLM backend (orchestration hedefi)
- `crates/ai/cave-mlx` — Apple Silicon array core (orchestration hedefi)

---
*Decided by Burak Tartan 2026-05-28; recorded by Claude (Opus 4.8), 2026-05-30.*
