# ADR-044: Supported Language Runtimes

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD / Build

**Related ADRs:** 010, 043, 076

## Context

CAVE's CI pipeline must support multiple programming languages. Golden Path templates (ADR-140 → Golden Path section) scaffold complete projects per language. Each language requires: build adapter (stage 8), test runner, schema migration tool (stage 6), and Backstage template.

## Candidates

| Approach | Phase 1 (Java + Python) — chosen | All 4 languages Phase 1 | Single language only | Custom per tenant |
|---|---|---|---|---|
| Build adapter maintenance | ✅ 2 adapters, manageable | ⚠️ 4 adapters from day 1 | ✅ 1 adapter | ❌ Per-tenant adapters |
| Template coverage | ✅ Covers ~80% of tenant needs | ✅ 100% coverage | ⚠️ Limits tenant adoption | ⚠️ Fragmented |
| CI pipeline reuse | ✅ Shared 27-stage pipeline | ✅ Shared | ✅ Shared | ❌ Custom per tenant |
| Team capacity | ✅ Sustainable | ❌ Spreads team too thin at launch | ✅ | ❌ Unsustainable |
| Golden Path discipline | ✅ Clear, opinionated | ⚠️ Diluted focus | ✅ | ❌ No golden path |

## Decision

**Phase 1 languages:** Java (Spring Boot) + Python (FastAPI). **Phase 2:** Go, Node.js/TypeScript. **Phase 3+:** Rust, .NET (on demand).

| Language | Build Tool | Test Runner | Schema Migration | Framework |
|---|---|---|---|---|
| Java | Maven/Gradle → Buildah | JUnit 5 | Flyway | Spring Boot |
| Python | pip → Buildah | pytest | Alembic | FastAPI |
| Go | go build → Buildah | go test | Atlas | stdlib/Fiber |
| Node.js | npm → Buildah | Jest/Vitest | Prisma | NestJS/Express |

Each language gets a Backstage scaffolder template that produces: Dockerfile (multi-stage), CI workflow (reusing shared 27-stage pipeline), Helm chart or Kustomize overlay, k6 load test scaffold, Flyway/Alembic migration directory.

## Rejected

- **Single language only (Java or Python):** Limits tenant adoption. Many data teams use Python, application teams use Java. Supporting both from Phase 1 maximizes platform utility.
- **All languages from Phase 1:** Each language requires dedicated template, build adapter, and CI testing. Spreading too thin. Phase 1 focuses on the two most common (Java + Python), Phase 2 adds Go and Node.js.
- **Language-specific CI pipelines:** Separate pipeline per language = maintenance explosion. CAVE uses shared 27-stage pipeline with language-specific build adapter only (stage 8).

## Implementation Reference

**Implementation Status:** Phase 1 complete (Java + Python), Phase 2 in progress (Go + Node.js)

- **cave-scaffold** crate: Backstage scaffolder templates per language
- **Build adapters:** Stage 8 of CI pipeline has language-specific Buildah invocation + dependency resolution (Maven for Java, pip for Python, go mod for Go, npm for Node.js)
- **Supported versions:** Java 11+, Python 3.9+, Go 1.19+, Node.js 16+ (LTS-only)

## Consequences

### Positive

- **Golden Path templates:** Backstage scaffolder creates complete project (Dockerfile, Helm chart, k6 tests, Flyway migrations) in <5min per language.
- **Standardized CI:** All 73 components + all tenant workloads run same 27-stage pipeline. Only stage 8 (build) changes per language.
- **Phase approach:** Phase 1 (Java + Python) covers ~70% of tenant use cases. Phases 2-3 add on-demand (Go, Node.js, Rust, .NET).
- **Developer experience:** Language-familiar build tools (Maven, pip, go, npm). No "forced" standardization to alien toolchains.

### Negative

- **Per-language maintenance:** Each language needs template + CI test coverage. 5 languages = 5 build adapters to maintain.
- **Phasing friction:** Tenants needing Go in Phase 1 must use "Expert Path" (manual pipeline setup). Workaround: contribute build adapter.
- **Language-specific knowledge:** Debug CI failures requires language expertise. Java GC issues different from Python import resolution issues.

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Unsupported language request blocks tenant | Medium | Low | Expert Path provides manual setup path. Roadmap (ADR-127) prioritizes new languages based on tenant demand. |
| Build adapter regression breaks all Java builds | Low | High | Staging tests all languages. Adapter changes require extensive CI testing. Runbook for quick rollback. |
| Language version EOL breaks builds | Medium | Low | Upgrade policy: Java LTS only (11, 17). Python 3.9+. Regular EOL scanning (Renovate ADR-041). |

## License

Build tools: Maven (Apache 2.0), Python (PSF), Go (BSD), Node.js (MIT)

## Compliance Mapping

**SOC2 CC8.1:** Standardized development practices — shared 27-stage pipeline enforces consistent build quality across languages.
**ISO/IEC 27001 A.8.25:** Secure development lifecycle — language-agnostic security scanning (SAST, SBOM, image scan) per language.
