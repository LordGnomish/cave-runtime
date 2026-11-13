# ADR-RUNTIME-DEV-MODE-001 — Cave Runtime Dual-Mode (Server + Dev)

**Title:** Cave Runtime Dual-Mode — Server (Cloud OS) + Dev (Developer Workstation)
**Status:** Accepted
**Scope:** Cave Runtime (single binary, all deployment targets)
**Category:** Charter / Architecture / Developer Experience
**Decided:** 2026-05-28 (Burak Tartan)
**Related ADRs:** ADR-RUNTIME-STACK-001 (stack layering), ADR-RUNTIME-CLI-CONSOLIDATION-001 (cavectl), ADR-RUNTIME-OPENJARVIS-ADOPTION-001 (local-first AI), ADR-001 (sovereign bare-metal hosting)

---

## Context

Cave Runtime şimdiye kadar yalnızca **production sovereign Cloud OS** olarak tasarlandı: multi-node HA, Charter v2 full compliance, Layer 1–4 unified Rust reimpl (ADR-RUNTIME-STACK-001). Bu konum doğru, ama eksik bir adoption ayağı var.

Burak'ın vizyonu: **aynı binary developer'ların local laptop'larında da çalışsın.** Dev'ler aynı manifest'lerle local'de geliştirip aynen prod'a deploy etsin — `minikube` / `kind` / `k3d` muadili ama Cave-native, ayrı bir tooling değil. Bu, ekosistemde kanıtlanmış bir adoption funnel'ı: developer önce laptop'ında çalıştırır, sonra prod'a taşır. Dev/prod parity bu funnel'ın temel taşı.

Tek node, lightweight, hot-reload edebilen bir mod olmadan Cave Runtime "datacenter-only" kalır ve developer benimsemesi düşer. Aynı zamanda local-first AI vizyonu (ADR-RUNTIME-OPENJARVIS-ADOPTION-001) developer workstation'ında bir runtime gerektiriyor.

## Decision

Tek binary, **iki deployment mode**:

- `cave-runtime --mode=server` (**default**) — multi-node HA, Charter v2 full compliance. Bugünkü davranış değişmez.
- `cave-runtime --mode=dev` — laptop-friendly, tek node, lightweight, hot-reload, opt-in component'ler.

**Yeni binary yok, yeni microservice topology yok, helm-per-crate yok** — `--mode` aynı `cave-runtime` artifact'ı üzerinde bir runtime flag'idir. Server mode'un superset davranışı dev mode'da seçmeli olarak kapatılır (graceful degradation).

### Dev mode mimarisi

1. **Component opt-in** — minimum çalışır çekirdek `apiserver + kubelet + cri`. Diğer Layer 3/4 component'leri (etcd-embedded, scheduler, net, gateway, vault, registry, streams, ...) explicit opt-in. Server mode'da hepsi default-on, dev mode'da default-off + profil ile açılır.
2. **Profile preset'ler:**
   - `minimal` (~4 GB RAM) — apiserver + kubelet + cri + embedded store. Salt manifest apply/test.
   - `standard` (~8 GB RAM) — minimal + scheduler + net + local registry + gateway. Tipik app geliştirme.
   - `full` (~16 GB+ RAM) — Charter v2 component set'in laptop'ta makul tüm kısmı. Prod-yakın doğrulama.
3. **Platform support:**
   - **Linux native** — birinci sınıf hedef (cgroups, namespaces, gerçek CRI).
   - **macOS / Windows fallback** — Layer 1/2 (Cave kernel + userspace) henüz yokken host OS üstünde lightweight VM veya containerd-muadili fallback datapath. Native parite hedefi değil, dev-loop hedefi.
4. **Hot-reload manifest watcher** — `~/.cave/manifests/` (veya `--watch <dir>`) izlenir, değişiklik diff'lenip reconcile edilir. Dev inner-loop için apply-bekle döngüsünü ortadan kaldırır.
5. **GPU access** — NVIDIA (CUDA) / AMD (ROCm) / Apple Silicon (Metal) passthrough; `cave-local-llm` (Ollama/vLLM), `cave-mlx`, ve OpenJarvis backend orchestration (ADR-RUNTIME-OPENJARVIS-ADOPTION-001) için. Donanım algılanır, uygun backend auto-config edilir.
6. **Persistent state** — `~/.cave/` altında: config, embedded store data, manifests, logs, GPU/backend cache. `cavectl dev reset` ile temizlenebilir.
7. **CLI UX** — `cavectl dev {up,down,status,logs,exec,shell,reset}`:
   - `up` / `down` — dev runtime'ı profil ile başlat/durdur.
   - `status` — component sağlığı + kaynak kullanımı.
   - `logs` — birleşik / component-filtreli log stream.
   - `exec` / `shell` — workload içine komut / shell.
   - `reset` — `~/.cave/` state temizle, temiz başlangıç.
8. **Install paths** — Homebrew (`brew install cave-runtime`), apt/dnf paketleri, winget, ve `curl | sh` install script. Hepsi aynı tek binary'i kurar.
9. **Editor entegrasyonu** — VS Code / Cursor / Zed için devcontainer template; `cave dev up` ile bağlanan reproducible dev environment.

## Consequences

### Positive
- **Dev/prod parity** — aynı binary, aynı manifest, aynı reconcile mantığı. "Bende çalışıyordu" sınıfı hatalar minimuma iner.
- **Adoption funnel** — laptop → staging → prod doğal yol; minikube/kind/k3d ekosisteminin Cave-native karşılığı.
- **OpenJarvis fit** — local-first personal AI (ADR-RUNTIME-OPENJARVIS-ADOPTION-001) developer workstation'ında bir runtime'a oturur; dev mode o runtime'dır.
- **Tek artifact disiplini korunur** — K3s pattern (112 crate → 1 binary) bozulmaz; `--mode` sadece runtime davranış flag'i.

### Negative
- **Test matrisi genişler** — server + dev × (minimal/standard/full) × (Linux/macOS/Windows). CI maliyeti artar.
- **Graceful degradation gerekli** — her component'in "ben yokken sistem makul davranır" yolunu desteklemesi gerekir; bazı server-mode invariant'ları (multi-node quorum vb.) dev mode'da anlamlı bir tek-node fallback'e indirgenmeli.
- **macOS/Windows fallback bakım yükü** — Layer 1/2 native olana kadar host-OS-spesifik datapath kodu taşınır (geçici, ama mevcut).

### Risks
| Risk | Mitigation |
|---|---|
| Dev mode "ayrı ürün" gibi yayılır, parity kaybolur | `--mode` aynı binary; ortak reconcile/codepath zorunlu, dev-only fork yasak. CI parity gate. |
| macOS/Windows fallback gerçek prod'dan saparak yanlış güven verir | `cavectl dev status` net "fallback datapath — not production-equivalent" uyarısı; full profile yalnızca Linux'ta tam parite iddia eder. |
| Component opt-in graceful degradation eksik → kırık dev deneyimi | Profile preset'leri test-edilmiş, bilinen-çalışan kombinasyonlar; rastgele opt-in kombinasyonu best-effort. |
| GPU passthrough donanım çeşitliliği | Vendor-agnostik detection; algılanamayan donanımda CPU fallback + net uyarı. |

## Alternatives considered

1. **Ayrı dev binary (`cave-dev`)** — *Reddedildi.* İki artifact = iki codepath = parity erozyonu; tek-binary disiplinini (K3s pattern) bozar.
2. **VM-wrapped dev distribution (her zaman bir VM içinde Linux Cave)** — *Reddedildi.* Ağır, yavaş inner-loop, GPU passthrough karmaşık, macOS/Windows'ta kötü DX. VM yalnızca native datapath olmayan platformlarda *fallback* olarak kullanılır, varsayılan model değil.
3. **Mevcut server mode'u laptop'ta çalıştır (mode yok)** — *Reddedildi.* Multi-node HA varsayımları, kaynak iştahı ve Charter v2 full compliance laptop'ta çalışmaz; lightweight bir profil katmanı şart.

## Implementation roadmap

Detaylı faz planı: [`docs/devmode/roadmap-2026-05-28.md`](../devmode/roadmap-2026-05-28.md)

- **Phase 1** — Profile system (`--mode=dev`, minimal/standard/full preset'ler, component opt-in iskeleti).
- **Phase 2** — `cavectl dev` suite (`up/down/status/logs/exec/shell/reset`).
- **Phase 3** — Hot-reload manifest watcher.
- **Phase 4** — Devcontainer template + install scripts (Homebrew/apt/dnf/winget/curl).
- **Phase 5** — GPU passthrough + AI backend auto-config (cave-local-llm / cave-mlx / OpenJarvis).

## Compliance / Cross-cutting
- **Tek artifact** (cave-runtime-single-binary): `--mode` bunu bozmaz; helm-per-crate ve microservice topology yasak.
- **Stack layering** (ADR-RUNTIME-STACK-001): Layer 1/2 native olana kadar dev mode host-OS fallback kullanır; bu mimari değil, geçici dev tooling.
- **CLI consolidation** (ADR-RUNTIME-CLI-CONSOLIDATION-001): `cavectl dev` alt-komut ailesi mevcut cavectl içine entegre, ayrı CLI değil.

## Related
- ADR-RUNTIME-STACK-001 — stack layering (Layer 1–4 tek owner)
- ADR-RUNTIME-CLI-CONSOLIDATION-001 — cavectl native + compat
- ADR-RUNTIME-OPENJARVIS-ADOPTION-001 — local-first personal AI (dev mode'un AI ayağı)
- `docs/devmode/roadmap-2026-05-28.md` — implementation faz planı

---
*Decided by Burak Tartan 2026-05-28; recorded by Claude (Opus 4.8), 2026-05-30.*
