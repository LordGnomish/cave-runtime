# ADR-034: Tenant Application Analytics — Umami

**Status:** Accepted

**Scope:** Universal

**Category:** Platform / Tenant Application Analytics

**Date:** 2026-04-26

**Related ADRs:** 029 (LGTM), 031 (WebApplication Composition), 037 (Unleash Feature Flags), 044 (Language Runtimes — Node.js Phase 1), 045 (k6 Load Testing), 047 (PostgreSQL Operator), ADR-RUNTIME-UI-CONSOLIDATION-001 (cave-portal wrap), ADR-RUNTIME-UPSTREAM-MIRROR-001 (mirror blanket)

## Context

Cave Platform tenant'ları kendi son-kullanıcı uygulamalarını (`cavectl deploy <app>` ile, ADR-031 WebApplication Composition Pattern üzerinden) Cave üzerine deploy eder. Bu uygulamaların **end-user analytics** ihtiyacı vardır:

- Page view sayımı (route + tarih).
- Conversion funnel (login → ilk işlem → checkout / aktivasyon).
- Session duration, bounce rate.
- Referrer (UTM, organic, direct).
- Coğrafi dağılım (ülke / şehir grain'i).
- Tarayıcı / cihaz / OS dağılımı.
- Custom event (`cart-added`, `signup-completed`, `feature-X-used`).

Cave Platform sovereignty + GDPR-friendly default + multi-tenant scoping invariant'ları bu ihtiyacı çözen aracın **kendi alanına ait** olmasını şart koşar:

1. **Sovereign:** Tenant analytics verisi 3rd-party SaaS'a (Google Analytics, Mixpanel, Amplitude, Plausible Cloud) **gönderilmez**. Veri Cave Platform içinde kalır.
2. **GDPR-friendly default:** Cookieless tracking + minimal PII (IP hash, no fingerprinting). Tenant cookie consent banner takmak zorunda kalmasın diye default cookieless.
3. **Multi-tenant scoping:** Tenant A, Tenant B'nin site verilerine erişemez. `tenant_id` propagation cave-portal seviyesinde default-deny.
4. **Cave stack alignment:** Mevcut cave-pg (ADR-047) + Node.js Phase 1 (ADR-044) ile uyum; ayrı runtime / DB ekleme yok.
5. **cave-portal native wrap (ADR-RUNTIME-UI-CONSOLIDATION-001):** Tenant Umami'nin kendi UI'sına gitmek zorunda değil; cave-portal içinde **Analytics** panel'inden, persona-aware (Tenant / Operator / Admin) dashboard görür.

Cave App Marketplace ileri-roadmap'inde Twenty CRM, Airflow gibi curated tenant uygulamalarının yanına web-analytics kategorisinde **bir** curated default lazım. Bu ADR onu seçer.

## Candidates

| Criteria | Umami | Google Analytics (GA4) | Plausible | PostHog | Matomo | Self-built |
|---|---|---|---|---|---|---|
| License | MIT | Proprietary SaaS | AGPL (self-host kısıtlı) | MIT (core) + Cloud upsell | GPL v3 | n/a |
| Sovereign self-host | ✅ Tek container + Postgres | ❌ SaaS only | ⚠️ Self-host olur, business model ufukta | ✅ Heavy stack (Kafka + ClickHouse + Postgres) | ✅ PHP + MySQL/MariaDB | ✅ ama scope explosion |
| Stack alignment | ✅ Next.js + Node.js + Postgres = cave-pg + ADR-044 | n/a | ✅ Elixir | ⚠️ ClickHouse + Kafka + Python | ❌ PHP / MySQL | n/a |
| GDPR-friendly default (cookieless) | ✅ Default cookieless | ❌ Cookie-heavy | ✅ Cookieless | ⚠️ Cookies on by default | ⚠️ Configurable | n/a |
| Multi-tenant native | ✅ Account/site model | ⚠️ Property-level | ⚠️ Site-level | ✅ Project-level | ⚠️ Site-level | n/a |
| API-first | ✅ REST + Send API | ✅ Measurement API | ⚠️ Limited | ✅ Comprehensive | ⚠️ Reporting only | n/a |
| Scope (web analytics niche) | ✅ Focused | n/a | ✅ Focused | ❌ Product analytics + flags + session replay (over-engineered for niche) | ⚠️ Heavy / many features | n/a |
| Operational footprint | ✅ Light (1 container + Postgres) | n/a | ✅ Light | ❌ Heavy | ⚠️ Medium | n/a |
| Conflict with existing Cave components | None | n/a | None | ⚠️ Çakışır: Unleash (ADR-037) feature flags, k6 (ADR-045) A/B testing | None | n/a |
| Maturity (2026-04) | 3.1.0+, stable | n/a | Stable | Stable | Mature | n/a |

## Decision

**Umami 3.1.0+** (umami-software/umami, MIT license) Cave Platform'ın **tenant application web-analytics** curated default'u olur.

### Tech stack uyumu

- **Next.js + Node.js 18+** — ADR-044 Phase 1 Node.js LTS run-time uyumu.
- **PostgreSQL** — cave-pg operator (ADR-047) backend; ayrı DB engine eklenmiyor.
- **Cookieless tracking default** — GDPR Art. 6 + Art. 25 (privacy by default).
- **Multi-tenant native** — Account → Site → Event hiyerarşisi tenant_id taşır; cave-portal seviyesinde default-deny.
- **API-first** — REST + Send API. cave-portal içinden native wrap; tenant Umami UI'sını **hiç görmez**.

### Cave entegrasyon yüzeyi

#### 1. Tenant deploy senaryosu (ADR-031)

WebApplication Composition Pattern altında tenant uygulamasını şu şekilde deploy eder:

- **CLI:** `cavectl deploy my-app --analytics umami`
- **UI:** cave-portal "Deploy" akışında "Include analytics" toggle (default ON).

Cave Crossplane composition arkada şunu yapar:

1. Tenant'ın namespace'inde Umami container + cave-pg `Database` resource yaratır (Umami şeması).
2. Tracking script (`/script.js`) tenant uygulamasının HTML head'ine **otomatik enjekte** edilir (Next.js / SPA / static-site adapters).
3. Site `website_id` cave-portal'da otomatik bind edilir; tenant manuel adım yapmaz.

#### 2. cave-portal native wrap (ADR-RUNTIME-UI-CONSOLIDATION-001)

Tenant **Umami UI'sına gitmez**. cave-portal içinde "Analytics" panel persona-aware dashboard:

- **Tenant rolü:** kendi site(ler)inin metrikleri (page view, funnel, geo, referrer).
- **Operator rolü:** kendi tenant ailesindeki tüm site'ları cross-view; per-tenant filter.
- **Admin rolü:** platform-wide aggregate (anonimleştirilmiş tenant breakdown); SLA / capacity planning girdisi.

Backend: cave-portal Umami REST API'sını proxy eder; her isteğe `tenant_id` assertion enjekte edilir; cross-tenant default-deny.

#### 3. Cave App Marketplace seed

Cave App Marketplace v0.1'de **curated app** kategorileri:

- **CRM:** Twenty (önceki ADR-031 örneği).
- **Workflow:** Airflow.
- **Analytics:** **Umami** (bu ADR).

Tenant marketplace'ten "Add Umami to my app" ile tek tıkla tracking enable eder.

#### 4. Multi-tenant scoping

- `tenant_id` label cave-portal seviyesinde her isteğe propagate edilir.
- Umami `account_id` ↔ Cave `tenant_id` 1:1 mapping (tenant başına dedicated account).
- Cross-tenant erişim default-deny; istisna ancak explicit cross-tenant grant ile (admin + audit-pqc imzalı).
- Tracking script CDN per-tenant subpath (`/{tenant_id}/script.js`) — tenant A'nın script'i tenant B'nin site_id'sini taşıyamaz.

## Rejected Options

### Google Analytics / GA4 — Sovereignty ihlali

SaaS-only, tenant verisi Google'a gider. GDPR Schrems II sonrası AB tenant'ları için kabusu büyütür (data transfer impact assessment her tenant için). Cave sovereignty invariant'ı ihlal.

### Plausible — Closed-source business model riski

Self-host olur (AGPL) ama Plausible Cloud'a doğru evrilen business model uzun vadede self-host feature gap riski yaratır. Curated default seçimi long-term stable upstream gerektirir; Plausible bu güvenceyi MIT bir alternatif (Umami) varken haklı kılmaz.

### PostHog — Over-engineered for the niche; çakışmalar

PostHog **product analytics** odaklı: feature flags + session replay + A/B testing + cohort analysis. Cave'de:

- **Feature flags:** ADR-037 Unleash zaten karar.
- **A/B testing:** ADR-045 k6 + Unleash kombinasyonu zaten yolda.
- **Session replay:** privacy-by-default Cave invariant'ıyla gerilim üretir; ek consent UX yükü getirir.

Web analytics niche'i için stack ağır (Kafka + ClickHouse + Postgres + Python worker'lar). İleride product-analytics ihtiyacı netleşirse **PostHog ikinci bir ADR** olarak eklenebilir; bu ADR onu kapatmaz.

### Matomo — Stack uyumsuz

Mature ama PHP / MariaDB-MySQL stack; Cave Phase 1 Node.js + PostgreSQL alignment'ından dışarı. Tracking pixel JS bundle'ı ağır, p99 page-load impact'i Umami'nin önünde. Operasyonel çevre PHP-FPM + ayrı DB → operational debt.

### Self-built analytics — Anti-pattern

Niche solved problem'i sıfırdan yazma scope explosion'ı. Mirror prensibi (ADR-RUNTIME-UPSTREAM-MIRROR-001) zaten upstream-mirror-then-improve yolunu garanti eder; baştan sıfır yazma yasaktır.

## Consequences

### Positive

- **Sovereign analytics** — tenant verisi Cave Platform içinde kalır.
- **GDPR-friendly default** — cookieless + minimal PII; tenant uygulamasının default'u GDPR-uyumlu.
- **Tenant lock-in yok** — Umami MIT, tenant istediğinde verisini export edebilir; Cave Platform içinde alternatif analytics ileride eklenirse veri taşınabilir.
- **Cave stack alignment** — cave-pg (ADR-047) + Node.js (ADR-044) ile birebir uyum; ek runtime / DB engine yok.
- **cave-portal tek pane wrap** — tenant Umami UI'sına hiç gitmez; persona-aware dashboard tek yüzey.
- **Cave App Marketplace seed** — Twenty + Airflow + **Umami** üçlüsüyle marketplace'in ilk curated katmanı belirlenmiş olur.
- **Self-improving telemetry** — cave-self-improver gece pump'ında Umami event ingest pipeline'ı runtime test corpus'una girer.

### Negative

- **Self-host operational yükü** — Postgres + Node.js servis + tracking script CDN. Mitigation: cave-pg + cave-portal + cave-cdn üçü zaten Cave Platform'da var; Umami sadece bunlara konuşur, ek operasyonel aksiyon minimal.
- **Umami v3 scaling limits** — büyük tenant'lar için Postgres tablo partitioning veya ClickHouse migration ileride değerlendirilebilir. v0.1'de tenant başına ortalama trafik tahminleri Postgres'i karşılar; sınır geldiğinde **kademeli ClickHouse migration** ayrı ADR ile.
- **Feature gap (PostHog)** — funnel / cohort / session replay yok. Mitigation: `Custom Event API` + Cave dashboards ile basic funnel; gerçek product analytics ihtiyacı çıkarsa **PostHog ikinci ADR** olarak eklenir, bu ADR'yi kapatmaz.
- **Tracking script p99 impact** — Umami script ~1 KB gzip; minimal. CDN cache + immutable header ile p99 impact ölçülebilir < 10 ms.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Umami upstream feature drift (rapid evolve) | Medium | Low | Pinned 3.1.x; quarterly upstream sweep; Cave wrap stable. |
| Postgres tablo büyümesi büyük tenant'ı etkiler | Medium | Medium | Tenant başına partitioning + retention policy default 13 ay; ClickHouse migration roadmap'te. |
| Cross-tenant data leak (multi-tenant scoping bug) | Low | High | cave-portal proxy seviyesinde tenant_id assertion zorunlu; integration test suite cross-tenant deny test pass. |
| Tracking script tenant uygulamasını yavaşlatır | Low | Low | CDN + immutable cache; bundle size budget < 2 KB gzip; CI gate. |
| Umami v3 → v4 breaking change | Medium | Medium | Mirror prensibi: cave-analytics crate Cave-side stabil API yüzeyi tutar; upstream breaking değişiklikler crate seviyesinde absorb edilir. |

## Implementation Reference

**Implementation Status:** Accepted; rollout phased.

- **Helm chart:** Umami official chart pinned; cave-platform chart repo'da.
- **Database:** cave-pg `Database` resource per tenant (Umami schema).
- **Tracking script:** cave-cdn üzerinden per-tenant subpath (`/{tenant_id}/script.js`); tenant_id'i bind eder.
- **cave-portal wrap:** Analytics panel; REST API proxy + tenant_id assertion + persona-aware view.
- **Crossplane composition:** WebApplication composition (ADR-031) `--analytics umami` flag'i bu hattı tetikler.
- **Telemetry:** Umami event ingest rate, page-view counter, error rate Prometheus'a (ADR-029 LGTM) remote-write.
- **Audit:** her admin call (account create, site create, data export) cave-audit-pqc imzalı log emit.

## Rollout Phasing

| Phase | Trigger | Onboarded | Gate |
|---|---|---|---|
| 1 — Bootstrap | Helm install in `cave-analytics` namespace, cave-pg DB ready | Operator only | Health check + smoke ingest 7 gün stabil. |
| 2 — Tenant pilot | İlk 3 pilot tenant `--analytics umami` flag'iyle deploy eder | Pilot tenant'lar | Cross-tenant deny integration test pass; cave-portal Analytics panel UX review. |
| 3 — Cave App Marketplace seed | Marketplace v0.1'de "Analytics" kategorisinde Umami curated default | Tüm tenant'lar opsiyonel ekleme | Marketplace add-flow p95 ≤ 60 sn (deploy + script inject + ilk event). |
| 4 — cave-self-improver entegrasyonu | Gece pump'ı Umami ingest pipeline'ını runtime test corpus'una alır | Cave-internal | Self-improver telemetry feed-back loop stabil. |
| 5 — Scale review | Tenant başına Postgres tablo büyüme metrikleri | Tüm tenant'lar | Partitioning policy enforced; ClickHouse migration roadmap kararı. |

## Definition of Done

- Umami chart cave-platform repo'da pinned ve staged cluster'da 7 gün stabil.
- Tenant `cavectl deploy <app> --analytics umami` ile deploy eder; tracking script otomatik inject + ilk event ≤ 60 sn.
- cave-portal Analytics panel persona-aware dashboard (Tenant / Operator / Admin) production'da.
- Cross-tenant deny integration test suite pass (cave-portal proxy + Umami account scope).
- Cave App Marketplace v0.1'de "Analytics → Umami" kategorisi yayında.
- Audit log emit (cave-audit-pqc) tüm admin event'leri için.
- cave-pg Umami şema retention policy default 13 ay; tenant override ile uzatılabilir (compliance retention).
- cave-self-improver gece pump'ı ingest pipeline'ını test corpus'una almış.

## Runtime Mirror

ADR-RUNTIME-UPSTREAM-MIRROR-001 blanket kuralı altında Runtime sovereign reimpl **`cave-analytics`** crate olarak yazılır:

- Single upstream → blanket scope; **ayrı Runtime override ADR yazılmaz**.
- Cave-side stable API yüzeyi (event ingest + report query); Umami upstream breaking değişiklikleri crate'te absorb edilir.
- cave-self-improver gece pump'ı `cave-analytics` test corpus'una Umami event ingest pipeline'ını dahil eder.
- Apache 2.0 lisansta tek crate; mirror prensibi kapsamı dışında ek özelleştirme dokümante edilir (PQC audit, multi-tenant default-deny invariant'lar).

## License

**Umami:** MIT — `https://github.com/umami-software/umami/blob/master/LICENSE`

MIT zaten Apache 2.0'dan daha permissive; ek alternatif değerlendirmesi gerektirmedi.

## Compliance Mapping

- **GDPR Art. 6 (lawfulness)** — cookieless default → consent gerekliliği minimum.
- **GDPR Art. 25 (privacy by design / by default)** — minimal PII (IP hash); tenant default'u GDPR-uyumlu.
- **SOC2 CC1.4** — Data residency tenant scoped; multi-tenant default-deny.
- **ISO/IEC 27001 A.5.34** — PII protection — minimal collection + retention policy default 13 ay.
- **NIS2 Art. 21** — Operational resilience; sovereign self-host bağımlılık zinciri kısa.

## Notes / Roadmap

- **Slot 034 reuse:** forensic'te bu slot ADR-035'e (Security Finding Aggregation — DefectDojo) absorbed olarak işaretliydi (decision text orada yaşıyor); bu ADR ile slot **canlanır**. Burak'ın "ezilme yok" kuralı korunur — ADR-035 mevcut DefectDojo kararı dokunulmaz; ADR-034 yeni Tenant Application Analytics konusu için açılır. **Semantic renumeration ileride yapılacak**, bu ADR sırasında değil.
- **Twenty CRM** (ADR-031 örnek senaryoda Airflow yanına eklenmişti) + **Umami** (bu ADR) + **Airflow** = Cave App Marketplace'in ilk curated üçlüsü.
- **PostHog ikinci ADR olarak roadmap'te tutulur** — product analytics + funnel + session replay ihtiyacı netleşirse, web analytics niche'inde Umami yerini korur, product analytics niche'i PostHog ile açılır.
- **ClickHouse migration** — büyük tenant scaling'i Postgres tablo partitioning'ini geçince ayrı ADR ile değerlendirilir.

## Related

- **ADR-031** — WebApplication Composition Pattern; tenant deploy senaryosu (`--analytics umami` flag) bu ADR üzerinden yürür.
- **ADR-044** — Supported Language Runtimes; Phase 1 Node.js LTS Umami stack uyumu.
- **ADR-047** — PostgreSQL Operator (cave-pg); Umami backend.
- **ADR-037** — Unleash Feature Flags; A/B testing roadmap'inde Umami custom event API ile birlikte kullanılır.
- **ADR-045** — k6 Load Testing; A/B test trafik üretiminde Umami tracking ile beraber çalışır.
- **ADR-029** — LGTM stack; Umami metrics Prometheus remote-write.
- **ADR-030** — OPA Gatekeeper; cave-portal Analytics proxy tenant_id assertion policy.
- **ADR-014** — Zero-Trust Network Architecture; her admin call cave-audit-pqc imzalı.
- **ADR-RUNTIME-UI-CONSOLIDATION-001** — cave-portal native wrap (Analytics panel persona-aware).
- **ADR-RUNTIME-UPSTREAM-MIRROR-001** — blanket mirror; cave-analytics crate ayrı override ADR olmadan kapsanır.
- **ADR-035** — Security Finding Aggregation (DefectDojo); slot 034 forensic'te bu ADR'a absorbed işaretliydi, bu ADR ile slot canlandı, ADR-035 dokunulmaz.

---
*Decided by Burak Tartan, recorded by Sonnet, 2026-04-26 ADR session.*
