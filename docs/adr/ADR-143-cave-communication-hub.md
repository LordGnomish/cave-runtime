# ADR-143 — Cave Communication Hub: Multi-Surface Team + LLM Chat Platform

**Status:** Proposed (Burak finalize edecek)
**Scope:** Universal (Cave Runtime — all tenants, all profiles)
**Category:** Communication / Collaboration / AI Workbench
**Related:** ADR-011 (Backstage Portal), ADR-013 (LiteLLM Gateway), ADR-014 (Zero-Trust), ADR-016 (Pod Security / Tetragon)
**Decided draft:** 2026-04-26 (Burak Tartan)

---

## 1. Context

Cave Runtime şu ana kadar üç ayrı communication ekseni taşıdı:

1. **Tenant team chat** — Slack / Mattermost / Rocket.Chat dış bağımlılıkla çözülüyordu (ADR-011 Portal'da link olarak).
2. **LLM workbench** — ad-hoc, ChatGPT-benzeri tek-kullanıcı arayüzü (ADR-013 LiteLLM gateway üstüne her tenant için bir sayfa).
3. **Operational notifications** — alert routing (Alertmanager → email/Slack), incident channel'ları, oncall handoff.

Bu üç eksen ayrıdır → context fragmantasyonu (incident kanalında konuşulan, LLM workbench'de tekrar yapıştırılır), audit boşluğu (LLM konuşmaları tenant audit log'unda yok), kompliyans riski (tenant verisi external SaaS'a sızıyor — GDPR Art. 28 sub-processor mayhem), federation imkansızlığı (tenant-A ↔ tenant-B mesajlaşması yok), PQC yol haritasıyla uyumsuzluk (Slack/Discord PQC E2EE roadmap ortada yok).

Aynı zamanda iki yeni ihtiyaç çıktı:
- **LLM as participant** — ajanlar (Claude, qwen3, gpt-oss-120b vs.) channel'a `@mention` ile çağrılmalı, threadleri okuyup tool calling yapabilmeli (cave-mcp-client üzerinden), audit'e işlemeli.
- **Surface çeşitliliği** — Portal web yetmiyor; CLI'dan (cavectl) chat, mobil push, desktop tray notification beklentisi var.

**Karar gereği:** Tüm communication'ı tek hub'da konsolide eden, runtime-native, multi-surface, LLM-first, E2EE+PQC ready, Matrix protokol uyumlu bir platform inşa edilir.

---

## 2. Decision

Cave Runtime, **Cave Communication Hub** adında bir runtime-native chat platformu sunar. Backend tek crate (`cave-chat`), Matrix client-server protokolünü Rust'ta reimpl eder (Synapse Python yok), 4 surface (Portal web + cavectl CLI + Mobile + Desktop) ile kullanılır, LLM'ler birinci sınıf participant'tır.

### 2.1 High-Level Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│  Surfaces                                                                │
│ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────────┐ │
│ │ Portal Web   │ │ cavectl CLI  │ │ Mobile       │ │ Desktop          │ │
│ │ (Yew/Leptos) │ │ (ratatui)    │ │ (iOS/Android │ │ (macOS/Win/Linux │ │
│ │              │ │              │ │  Tauri)      │ │  Tauri)          │ │
│ └──────┬───────┘ └──────┬───────┘ └──────┬───────┘ └────────┬─────────┘ │
└────────┼────────────────┼────────────────┼──────────────────┼───────────┘
         │ Matrix client-server (HTTPS+WS) + cave-auth tokens │
         ▼                ▼                ▼                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  cave-chat (Rust, Matrix C-S protocol reimpl)                            │
│ ┌────────────────────┐ ┌──────────────────┐ ┌─────────────────────────┐ │
│ │ Room state machine │ │ Sync stream      │ │ Federation (S2S Matrix) │ │
│ │ (DAG, ACLs, power) │ │ (long-poll/SSE)  │ │ (PQC-hybrid TLS)        │ │
│ └────────────────────┘ └──────────────────┘ └─────────────────────────┘ │
│ ┌────────────────────┐ ┌──────────────────┐ ┌─────────────────────────┐ │
│ │ E2EE Olm/Megolm    │ │ Bot/LLM dispatch │ │ Media + RAG ingest      │ │
│ │ + PQC hybrid       │ │ (cave-mcp-client │ │ (cave-iceberg)          │ │
│ │ (ML-KEM + ML-DSA)  │ │  per LLM agent)  │ │                         │ │
│ └────────────────────┘ └──────────────────┘ └─────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────────┘
         │                │                │                  │
         ▼                ▼                ▼                  ▼
┌─────────────┐ ┌──────────────┐ ┌─────────────────┐ ┌────────────────────┐
│ cave-pg     │ │ cave-docdb   │ │ cave-iceberg    │ │ cave-streams       │
│ rooms,users,│ │ messages,    │ │ files, attach-  │ │ presence, typing,  │
│ memberships │ │ events DAG   │ │ ments, RAG idx  │ │ read receipts      │
└─────────────┘ └──────────────┘ └─────────────────┘ └────────────────────┘
         │                                  │                   │
         ▼                                  ▼                   ▼
   ┌──────────────┐                 ┌──────────────┐   ┌──────────────────┐
   │ cave-search  │                 │ cave-audit   │   │ cave-litellm     │
   │ (FT search,  │                 │ (PQC-signed  │   │ (LLM completion  │
   │  Tantivy)    │                 │  append-log) │   │  + tool calling) │
   └──────────────┘                 └──────────────┘   └──────────────────┘
```

Tüm bağımlılıklar **Cave Runtime crate'leri**dir; external SaaS yoktur.

### 2.2 Why Matrix Protocol

- **Açık standart** (Matrix.org Foundation), **federation tasarımda var**.
- Olm/Megolm E2EE olgun (Element kullanıyor, FOSDEM/NLnet PQC araştırmalarıyla uyumlu).
- Room state DAG, ACL ve power-level semantiği ihtiyacımızı karşılıyor (Slack-tarzı channel + Discord-tarzı role + Matrix space hiyerarşisi tek primitif).
- **Synapse (Python referans implementasyon) reddedildi** — performans, supply chain, runtime-native politika gereği. **Dendrite (Go) reddedildi** — ekosistem Go bağımlılığı eklemek istemiyoruz. Kendi Rust reimpl: `cave-chat`.

---

## 3. Core Features (≥12)

| # | Feature | v0.1 | v0.2 | v0.3 |
|---|---|:---:|:---:|:---:|
| 1 | Channel (room) — public/private, hierarchical (Matrix Spaces) | ✓ | | |
| 2 | Direct Message (1:1) ve Group DM (≤8 kişi) | ✓ | | |
| 3 | Threaded replies (Matrix m.thread) | ✓ | | |
| 4 | Reactions, mentions (`@user`, `@channel`, `@here`, `@llm-name`) | ✓ | | |
| 5 | File upload (any MIME), inline preview (image/PDF/code) | ✓ | | |
| 6 | Full-text search (cave-search/Tantivy, per-tenant index) | ✓ | | |
| 7 | Message edit/delete history (Matrix m.replace), audit-preserved | ✓ | | |
| 8 | Presence (online/idle/dnd), typing indicators, read receipts | ✓ | | |
| 9 | LLM as participant (mention + DM, tool calling streaming) | ✓ | | |
| 10 | RAG over channel history + uploaded files (cave-iceberg) | partial | ✓ | |
| 11 | E2EE Olm/Megolm (classical X25519+Ed25519) | ✓ | | |
| 12 | E2EE PQC hybrid (ML-KEM-768 + ML-DSA-65 layered on Olm/Megolm) | | ✓ | |
| 13 | Federation (tenant↔tenant, opt-in per room) | | ✓ | |
| 14 | Mobile push notifications (APNs/FCM via cave-push relay) | | ✓ | |
| 15 | Desktop tray + system notifications | | ✓ | |
| 16 | Voice/video (1:1 + group ≤25, WebRTC SFU via `cave-sfu`) | | | ✓ |
| 17 | Screen share + collaborative cursor (CRDT, automerge) | | | ✓ |
| 18 | MCP client (full Anthropic spec — resources, tools, prompts, sampling) | partial | | ✓ |

`partial` = scaffold mevcut, full feature parity sonraki sürümde.

---

## 4. LLM Integration

### 4.1 LLM as Participant Model

LLM'ler runtime'da **first-class user**'dır:
- Her LLM bir `@<name>:<tenant>.cave.local` Matrix user ID alır (örn. `@qwen3:acme.cave.local`).
- Matrix user record'u `cave-pg.users` tablosunda `user_kind = 'llm'` flag'i ile işaretlenir.
- **Authentication:** her LLM identity için `cave-vault` üzerinde Ed25519 device key (PQC eşleniği ML-DSA-65) saklanır; LLM dispatch worker bu key ile Matrix C-S session açar.
- **Authorization:** RBAC tabloda LLM user'ları normal user'lar gibidir; channel admin LLM'i invite/kick edebilir, role atayabilir (örn. `llm:read-only`, `llm:tool-caller`, `llm:moderator`).

### 4.2 Mention & DM Trigger

```
User mesajı → cave-chat parse → mention extraction
                                       │
                              ┌────────┴────────┐
                              │ @user mention?  │ → presence push, no LLM
                              ├─────────────────┤
                              │ @<llm> mention? │ → llm-dispatch event
                              ├─────────────────┤
                              │ DM to @<llm>?   │ → llm-dispatch event
                              └─────────────────┘
                                       │
                                       ▼
                            ┌─────────────────────────┐
                            │ cave-chat-llm-worker    │
                            │ (per-tenant pool)       │
                            └────────┬────────────────┘
                                     │
                                     ▼
              ┌───────────────────────────────────────────┐
              │ 1. Fetch context (last N events, RAG hit) │
              │ 2. Build messages array (role/content)    │
              │ 3. POST cave-litellm /v1/chat/completions │
              │    with tools=[mcp_tool_manifest], stream │
              │ 4. Stream chunks → m.room.message edits   │
              │    (Matrix `m.in_reply_to` + `m.replace`) │
              │ 5. On tool_call: dispatch to              │
              │    cave-mcp-client → tool execution       │
              │    → tool_result event in same thread     │
              │ 6. Loop (4-5) until completion            │
              │ 7. Audit: append signed event to          │
              │    cave-audit (PQC ML-DSA-65 signature)   │
              └───────────────────────────────────────────┘
```

### 4.3 Streaming Semantics

- LLM cevabı bir Matrix event olarak **placeholder** ile yaratılır (`content: { msgtype: "m.text", body: "…", "io.cave.streaming": true }`).
- Her chunk için `m.replace` event'i yayınlanır (Matrix native edit semantics).
- Surface'lar `io.cave.streaming` flag'i true iken edit'leri append olarak render eder; flag false olunca finalize ederler.
- Tool call sırasında ayrı bir thread event'i (`io.cave.tool_invocation`) doğar — tool name, arguments, result tüm participantlar için görünür (privacy: tool result redaction policy ayar edilebilir).

### 4.4 Tool Calling

- `cave-mcp-client` Anthropic MCP spec implementasyonu (resources / tools / prompts / sampling).
- Tenant per-LLM tool manifest cave-pg `llm_tool_grants` tablosunda; channel admin "bu LLM bu kanalda hangi tool'lara erişebilir" granular ayarı yapar.
- Dangerous tool'lar (`fs.write`, `shell.exec`, `network.fetch`) için **per-call human-in-the-loop confirmation** opsiyonel (kanal politikası).
- Tool çağrıları cave-audit'e PQC-signed olarak yazılır; rate limit per-LLM-per-tool.

### 4.5 LLM Backend

Tüm LLM çağrıları `cave-litellm` (ADR-013) üzerinden yapılır → tek choke point, observability, cost attribution. Cave Communication Hub LLM model seçimi yapmaz, tenant'ın `cave-litellm` model alias'larına göre gönderir.

---

## 5. Multi-Tenant Isolation

```
┌─────────────────────────────────────────────────────────┐
│ Tenant A                         Tenant B               │
│ ┌─────────────────────┐          ┌─────────────────────┐│
│ │ cave-chat namespace │          │ cave-chat namespace ││
│ │ rooms: !x:a.local   │          │ rooms: !y:b.local   ││
│ │ users: @u:a.local   │          │ users: @u:b.local   ││
│ │ media: s3://a-chat  │          │ media: s3://b-chat  ││
│ │ db   : pg-tenant-a  │          │ db   : pg-tenant-b  ││
│ └─────────────────────┘          └─────────────────────┘│
│         │ federation opt-in (mTLS PQC, per-room ACL)    │
│         └────────────────────►   federated room         │
└─────────────────────────────────────────────────────────┘
```

- **Logical:** her tenant kendi Matrix homeserver namespace'ine sahip (`<tenant>.cave.local`).
- **Storage:** cave-pg, cave-docdb, cave-iceberg per-tenant database/schema/bucket; cross-tenant query yasak (DB user ACL).
- **Network:** Cilium NetworkPolicy ile tenant pod'ları arası direct trafik yok; federation gateway üzerinden (mTLS, kanal başına ACL).
- **Encryption keys:** Olm/Megolm device key'leri tenant'ın `cave-vault` namespace'inde; cross-tenant key access yok.
- **Audit:** her tenant'ın audit log'u kendi cave-audit shard'ına; cross-tenant audit visibility ihlal sayılır.

---

## 6. RBAC

Matrix native power-level (0-100) modelinin üstüne Cave-specific role mapping:

| Role | Power Level | Yetki |
|---|:---:|---|
| `room:owner` | 100 | room delete, federation toggle, encryption upgrade |
| `room:admin` | 75 | invite/kick, role assign, retention policy, LLM grant |
| `room:moderator` | 50 | message redact, user mute, pin |
| `room:member` | 10 | post, react, edit own |
| `room:guest` | 0 | read-only, no post |
| `llm:tool-caller` | 25 | post + invoke granted tools |
| `llm:read-only` | 5 | read context, post text only, no tools |
| `bot:notifier` | 5 | post via cave-streams alert pipeline |

RBAC kararları **cave-rbac** crate (mevcut) tarafından enforce edilir; cave-chat sadece query atar.

---

## 7. Persistence Layers

| Layer | Crate | Ne tutulur | Wire / Format |
|---|---|---|---|
| Relational (rooms, users, memberships, ACL, power-levels) | `cave-pg` | Matrix room state, user identities, device list | PostgreSQL |
| Event DAG (messages, edits, redactions, reactions) | `cave-docdb` | Matrix events (JSON) — high write volume, sparse | MongoDB wire |
| Files & RAG | `cave-iceberg` | Uploaded blobs, derived embeddings, FT shards | S3 (Iceberg tables) |
| Search | `cave-search` | Full-text inverted index per-tenant per-room | Tantivy embedded |
| Presence / typing / read receipts | `cave-streams` | Ephemeral, TTL'd | NATS-compat wire |
| Audit | `cave-audit` | Signed append-log (PQC ML-DSA-65) | Append-only |
| Secrets (device keys, signing keys) | `cave-vault` | E2EE key material per device | KMS |

**Why split cave-pg vs cave-docdb?** Matrix room state mutating, ACID, cross-row transactional → relational. Event DAG append-heavy, schemaless variants (m.room.message, m.reaction, m.redaction, custom event types) → document store more natural. Cave kernel WAL ve Raft her iki crate'te paylaşımlı (ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001).

---

## 8. Encryption

### 8.1 Transport

- **Surface ↔ cave-chat:** TLS 1.3, hybrid Kyber768+X25519 (ADR-014 Zero-Trust gereği). PQC-only mode v0.3'te opsiyonel.
- **Server ↔ Server (federation):** mTLS, PQC-hybrid mandatory.

### 8.2 At-Rest

- cave-pg, cave-docdb: native at-rest encryption (AES-256-GCM, per-tenant key derivation).
- cave-iceberg blob'ları: S3 SSE-C, key cave-vault'tan.
- cave-audit: signed log + cipher-stream (record-level encryption).

### 8.3 End-to-End (E2EE)

**v0.1 — Classical Olm/Megolm:**
- Olm (1:1): Double Ratchet (X3DH key agreement, X25519 + Ed25519, AES-256-GCM + HMAC-SHA-256).
- Megolm (group): Forward-secret group ratchet, per-session Ed25519 fingerprint.
- Key backup: per-user passphrase-encrypted backup → cave-vault.

**v0.2 — PQC Hybrid:**
- **ML-KEM-768** (NIST FIPS 203, eski adı Kyber768): KEM, X3DH'nin DH adımına paralel layered (hybrid: classical XOR pq).
- **ML-DSA-65** (NIST FIPS 204, eski adı Dilithium3): device signing key (Ed25519 ile dual-sign).
- Megolm session key dağıtımında ML-KEM-768 kapsülü; out-of-band attack yüzeyi PQC korumalı.
- Backwards compatibility: PQC-yetersiz client ile classical fall-back izinli (room admin policy ile zorlama mümkün).

**Karar:** Slack/Discord'un yapamadığı PQC-hybrid E2EE Cave için diferansiyel — savunma sanayi, finans, kamu tenant'ları için satış argümanı.

---

## 9. Federation

### 9.1 Model

- Matrix Server-Server protokolü (S2S API) reimpl edilir.
- **Federation per-room opt-in.** Default: tenant-internal only. Room owner federation flag'ini açar, hangi remote tenant ID'lere açık olduğunu beyaz listeler.
- Cross-tenant federation cave-mesh (Cilium ClusterMesh) üzerinden mTLS PQC-hybrid; federation gateway pod'u tenant başına bir replica.

### 9.2 Identity Federation

- Remote user ID `@user:other-tenant.cave.local` formatında.
- Local cache: federated user metadata + device key signature chain cave-pg `federated_users` tablosunda.
- Trust: tenant root signing key cave-vault'ta; federation handshake'de signature chain verify.

### 9.3 Compliance Flags

- Room owner federation açtığında **GDPR data export** sorumluluğu remote tenant'a bildirilir (federation event metadata içinde).
- Federation kapatıldığında remote message'ların local copy'si 30-gün retention sonrası purge.

---

## 10. Surface Implementations

### 10.1 Portal Web (`cave-chat-portal-ui`)

- Yew (Rust→WASM) veya Leptos — kararlanmamış (ADR-PORTAL-* uyumlu olmalı).
- Embed olarak Backstage Portal'a iframe; standalone modu da var.
- Matrix C-S long-poll + WebSocket (sync_v3).
- E2EE: vodozemac (libolm Rust port) WASM bundle.
- File upload: chunked, resumable (tus.io spec).

### 10.2 cavectl CLI (`cave-chat-cli`)

- ratatui (terminal UI) + clap subcommand (`cavectl chat`).
- REPL: `cavectl chat repl <room>` — interactive.
- Scriptable: `cavectl chat post --room <id> --text "..."`, `cavectl chat watch --room <id> --json`.
- LLM mention shortcut: `cavectl chat ask @qwen3 "<question>"` → DM yaratır, cevabı stream eder, terminale.
- E2EE: vodozemac native (Rust binary).
- Notification: terminal bell + (opsiyonel) `notify-send`/`osascript`.

### 10.3 Mobile (`cave-chat-mobile`)

- **Tauri 2.0** (mobil destek) — tek codebase iOS + Android.
- Background fetch: APNs (iOS) / FCM (Android) push notification — payload encrypted, server sadece "yeni mesaj var" sinyali; içerik açılınca client çözer.
- E2EE: vodozemac via Tauri Rust core.
- Biometric unlock (Face ID / Fingerprint) device key'i unlock için.
- Offline queue: outbox cave-streams uyumlu, online olduğunda flush.

**Push notification gateway crate:** `cave-push` (yeni) — APNs/FCM credentials cave-vault'ta, cross-tenant izolasyon zorunlu (her tenant kendi APNs team ID'si).

### 10.4 Desktop (`cave-chat-desktop`)

- Tauri 2.0 — macOS / Windows / Linux.
- System tray + native notification.
- Auto-update: cave-update servisi (TUF spec).
- E2EE: vodozemac via Tauri Rust core (mobile ile aynı kütüphane).
- Global hotkey (örn. `Cmd+Shift+K` → quick-switcher).
- (Opsiyonel) Voice/video v0.3 — WebRTC, cave-sfu üzerinden.

---

## 11. Crates

### 11.1 Backend

| Crate | Sorumluluk |
|---|---|
| `cave-chat` | Matrix C-S + S2S protokol, room state machine, sync stream, federation |
| `cave-chat-llm` | LLM dispatch worker, mention parser, streaming aggregator |
| `cave-mcp-client` | Anthropic MCP client (resources/tools/prompts/sampling) |
| `cave-push` | APNs + FCM relay, encrypted-payload gateway |
| `cave-sfu` | WebRTC SFU (v0.3) — voice/video |

### 11.2 Surface

| Crate | Sorumluluk |
|---|---|
| `cave-chat-portal-ui` | Yew/Leptos web client, Backstage embed |
| `cave-chat-cli` | cavectl chat subcommand, ratatui REPL |
| `cave-chat-mobile` | Tauri iOS/Android wrapper |
| `cave-chat-desktop` | Tauri macOS/Windows/Linux wrapper |

### 11.3 Shared

| Crate | Sorumluluk |
|---|---|
| `cave-chat-proto` | Matrix event types, JSON schemas, error codes — surface + backend ortak |
| `cave-vodozemac` | libolm Rust port (upstream Matrix.org) thin wrapper, PQC layer hook |
| `cave-pqc` | ML-KEM-768 + ML-DSA-65 wrapper (liboqs Rust binding) |

Toplam yeni crate sayısı: **11**. Mevcut `cave-pg`, `cave-docdb`, `cave-iceberg`, `cave-search`, `cave-streams`, `cave-audit`, `cave-vault`, `cave-rbac`, `cave-litellm` reuse.

---

## 12. Rejected Alternatives

| Aday | Red sebebi |
|---|---|
| **Synapse (Matrix Python)** | Performans (GIL), supply chain (PyPI), runtime-native politika |
| **Dendrite (Matrix Go)** | Go ekosistem bağımlılığı eklemek istemiyoruz; tek dil (Rust) hedefi |
| **Mattermost / Rocket.Chat self-host** | LLM-first değil, federation Matrix kalitesinde değil, PQC roadmap yok |
| **Slack/Discord SaaS** | Tenant veri sızıntısı, GDPR/NIS2 ihlal riski, audit boşluğu, federation yok, PQC yok |
| **XMPP** | E2EE (OMEMO) Matrix Olm kadar olgun değil, multi-device sync zayıf, federation tooling fragmented |
| **IRC** | Modern feature set yok (file, presence, threads), E2EE yok |
| **Signal protocol direkt** | Multi-device key transparency operationally zor, federation yok, server scale-out modeli weak |
| **Kendi proprietary protokol** | Standartlaşma yok, federation imkansız, ekosistem (Element, vb.) kaybı |
| **Electron desktop** | RAM/CPU footprint, supply chain (Node), Tauri zaten Rust + smaller |
| **React Native mobile** | JS bridge overhead, Tauri 2.0 ile hem web hem mobile tek dil |

---

## 13. Consequences

### 13.1 Positive

- **Konsolidasyon:** team chat + LLM workbench + ops notifications tek surface; context fragmantasyonu biter.
- **Veri egemenliği:** tenant verisi external SaaS'a gitmez (GDPR Art. 28 sub-processor sayısı azalır).
- **PQC differential:** kuantum-dirençli E2EE 2026'da ticari ürünlerde nadir → satış argümanı (kamu, finans, savunma).
- **Federation native:** tenant↔tenant chat, B2B ortaklık entegrasyonu Matrix protokolünden geliyor.
- **LLM first-class:** ajan workflow'ları doğal — channel'da LLM'i mention et, tool çağırsın, audit'e işlesin.
- **Audit tek noktada:** mesaj + LLM çağrı + tool invocation aynı log'da, kompliyans için altın.

### 13.2 Negative

- **İnşa maliyeti yüksek:** Matrix C-S + S2S + Olm/Megolm + PQC + 4 surface = 11 yeni crate, ~6-9 ay engineering.
- **Karmaşıklık:** Matrix event DAG, room state resolution v2 algoritması, federation eventual consistency — runtime ekibi öğrenme eğrisi.
- **PQC olgunluk riski:** ML-KEM/ML-DSA bindings Rust'ta hâlâ değişiyor (liboqs version churn) — v0.2 timeline kayma riski.
- **Mobile distribution:** App Store / Play Store onayı tenant başına ayrı build/sign — operasyonel yük.
- **WebRTC SFU (v0.3):** voice/video kalite mühendisliği uzun kuyruk; Mediasoup/Janus reddederek kendi yazmak risk.

### 13.3 Risks

| Risk | Olasılık | Etki | Mitigation |
|---|:---:|:---:|---|
| Matrix protokol spec değişir, breaking | Orta | Orta | spec snapshot pin'le, kontrollü upgrade |
| PQC standartları (NIST) revize | Düşük | Yüksek | Hybrid mode (classical fallback) v0.2 default |
| Mobile Tauri 2.0 store onay sürtünmesi | Orta | Düşük | Native fallback (Swift/Kotlin) yedek plan |
| Federation abuse (spam, takeover) | Yüksek | Orta | Per-room opt-in default kapalı, allowlist, rate limit |
| LLM tool calling abuse (RCE-benzeri) | Orta | Yüksek | Per-tool grant + dangerous tool human confirm + audit |
| E2EE key loss → mesaj kaybı | Yüksek | Yüksek | Server-side encrypted backup (passphrase-derived) opsiyonel |

---

## 14. Implementation Phases

### v0.1 — Backend MVP + Portal + CLI (hedef: ~2026-08)

- [x] cave-chat scaffold (Matrix C-S subset)
- [x] cave-chat-proto schemas
- [x] Persistence wiring (cave-pg + cave-docdb + cave-search)
- [x] Auth integration (cave-auth, OIDC)
- [x] cave-chat-portal-ui MVP (text only, no E2EE)
- [x] cave-chat-cli (cavectl chat REPL)
- [x] LLM as participant (mention + DM, cave-litellm streaming)
- [x] Tool calling (cave-mcp-client subset, top-N tools)
- [x] cave-audit signed log integration
- [x] E2EE Olm/Megolm classical (Portal + CLI)

### v0.2 — Mobile + Desktop + Federation + PQC (hedef: ~2026-12)

- [ ] cave-chat-mobile (iOS+Android Tauri)
- [ ] cave-chat-desktop (macOS+Win+Linux Tauri)
- [ ] cave-push (APNs + FCM gateway)
- [ ] Federation (S2S Matrix, mTLS PQC)
- [ ] cave-pqc + vodozemac PQC layer (ML-KEM-768 + ML-DSA-65 hybrid)
- [ ] RAG full (cave-iceberg embedding ingest, retrieval per-channel)
- [ ] Push notification encrypted payload
- [ ] Compliance reports (GDPR data export, audit query API)

### v0.3 — Voice/Video + MCP Full (hedef: ~2027-Q2)

- [ ] cave-sfu (WebRTC SFU)
- [ ] Voice/video 1:1 + group (≤25)
- [ ] Screen share + collaborative cursor (CRDT)
- [ ] cave-mcp-client full Anthropic spec parity (resources, prompts, sampling)
- [ ] Voice transcription (cave-whisper integration)

---

## 15. Compliance

### 15.1 GDPR (EU 2016/679)

- **Art. 5 (data minimization):** mesaj retention policy per-room (default 365 gün, owner ayarlar).
- **Art. 17 (right to erasure):** user delete → cave-chat purge user'ın tüm mesajları (m.redaction event, cave-audit'e silme kanıtı).
- **Art. 20 (data portability):** `cavectl chat export --user <id> --format json` — Matrix event JSON, attachment'lar S3 link'i.
- **Art. 28 (sub-processor):** zero external sub-processor; APNs/FCM hariç (ayrı DPA).
- **Art. 32 (security of processing):** E2EE + PQC + signed audit + tenant isolation.

### 15.2 SOC2 (Trust Services Criteria)

- **Security:** RBAC + cave-audit + PQC-signed log immutability.
- **Availability:** cave-chat HA (Raft replication via cave-kernel), SLO ≥ 99.9% v0.1.
- **Processing Integrity:** Matrix event signature chain + cave-audit signature.
- **Confidentiality:** E2EE default for private rooms.
- **Privacy:** GDPR controls reuse.

### 15.3 ISO/IEC 27001

- A.5 (information security policies): cave-chat policy registry'de.
- A.8 (asset management): per-tenant data inventory cave-iceberg metadata.
- A.10 (cryptography): NIST FIPS 203/204 (PQC), FIPS 140-3 cave-vault.
- A.12 (operations): cave-audit + cave-streams ops trace.
- A.18 (compliance): GDPR/NIS2 cross-mapping documented.

### 15.4 NIS2 (EU 2022/2555)

- **Art. 21 (risk management):** PQC-readiness, multi-tenant isolation, federation rate limiting.
- **Art. 23 (incident reporting):** cave-chat incident channel template + 24h notification flow Cave Runtime'da default.

### 15.5 HIPAA (US 45 CFR §164)

- **§164.312(a) access control:** RBAC + MFA (cave-auth).
- **§164.312(b) audit:** cave-audit PQC-signed.
- **§164.312(c) integrity:** Matrix event signature chain.
- **§164.312(e) transmission security:** E2EE.
- **PHI handling:** healthcare tenant template room policy `phi-restricted` (federation off, retention strict, RAG opt-out).

---

## 16. Açık Sorular (Burak finalize edecek)

1. **Portal stack:** Yew vs Leptos — Backstage embed entegrasyonu için hangisi daha düşük WASM bundle?
2. **vodozemac vs custom Olm port:** vodozemac (Matrix.org official) kullan, yoksa Cave-internal fork?
3. **liboqs vs RustCrypto PQC:** ML-KEM/ML-DSA için hangi kütüphane? RustCrypto pure-Rust ama olgun değil; liboqs C-bindings ama olgun.
4. **WebRTC SFU:** Mediasoup-rs (port) vs ground-up `cave-sfu`? v0.3 timeline implication.
5. **Mobile App Store dağıtımı:** tek "Cave Chat" app her tenant'a multi-server seçici, yoksa tenant-başına white-label app?
6. **Federation default policy:** allowlist (default kapalı) vs blocklist (default açık) — güvenlik vs UX trade-off.
7. **LLM identity model:** her tenant'ın kendi LLM user'ları, yoksa global Cave LLM marketplace (cross-tenant erişimle)?
8. **MCP client tool grant UI:** channel admin tool'ları manuel yönetir, yoksa policy-as-code (OPA Rego)?
9. **cave-search engine:** Tantivy embedded (per-shard) vs Quickwit standalone (full search service)?
10. **Voice/video encryption:** SFU media path E2EE (DTLS-SRTP per-participant key) — performans-encryption trade-off.

---

## 17. References

- **Matrix Specification:** https://spec.matrix.org/
- **Olm/Megolm spec:** https://gitlab.matrix.org/matrix-org/olm/-/tree/master/docs
- **NIST FIPS 203 (ML-KEM):** kuantum-dirençli KEM standardı
- **NIST FIPS 204 (ML-DSA):** kuantum-dirençli digital signature standardı
- **Anthropic MCP:** https://modelcontextprotocol.io/
- **vodozemac (Rust Olm port):** https://github.com/matrix-org/vodozemac
- **liboqs:** https://github.com/open-quantum-safe/liboqs
- **Tauri 2.0 mobile:** https://v2.tauri.app/start/prerequisites/
- **ADR-011:** Backstage as Developer Portal
- **ADR-013:** LiteLLM as Unified LLM Gateway
- **ADR-014:** Zero-Trust Network Architecture
- **ADR-016:** Container Runtime Security (Pod Security / Tetragon)
