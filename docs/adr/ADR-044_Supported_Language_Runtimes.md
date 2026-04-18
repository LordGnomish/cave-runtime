# ADR-044: Supported Language Runtimes

**Status:** Accepted

**Category:** CI/CD

**Related ADRs:** 010, 043

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE's CI pipeline must support multiple programming languages. Golden Path templates (ADR-140 → Golden Path section) scaffold complete projects per language. Each language requires: build adapter (stage 8), test runner, schema migration tool (stage 6), and Backstage template.

## Candidates

## | Approach | Phase 1 (Java + Python) — chosen | All 4 languages Phase 1 | Single language only | Custom per tenant |
|---|---|---|---|---|
| Build adapter maintenance | ✅ 2 adapters, manageable | ⚠️ 4 adapters from day 1 | ✅ 1 adapter | ❌ Per-tenant adapters |
| Template coverage | ✅ Covers ~80% of tenant needs | ✅ 100% coverage | ⚠️ Limits tenant adoption | ⚠️ Fragmented |
| CI pipeline reuse | ✅ Shared 27-stage pipeline | ✅ Shared | ✅ Shared | ❌ Custom per tenant |
| Team capacity | ✅ Sustainable | ❌ Spreads team too thin at launch | ✅ | ❌ Unsustainable |
| Golden Path discipline | ✅ Clear, opinionated | ⚠️ Diluted focus | ✅ | ❌ No golden path |

## Decision

## **Phase 1 languages:** Java (Spring Boot) + Python (FastAPI). **Phase 2:** Go, Node.js/TypeScript. **Phase 3+:** Rust, .NET (on demand).

| Language | Build Tool | Test Runner | Schema Migration | Framework |
|---|---|---|---|---|
| Java | Maven/Gradle → Buildah | JUnit 5 | Flyway | Spring Boot |
| Python | pip → Buildah | pytest | Alembic | FastAPI |
| Go | go build → Buildah | go test | Atlas | stdlib/Fiber |
| Node.js | npm → Buildah | Jest/Vitest | Prisma | NestJS/Express |

Each language gets a Backstage scaffolder template that produces: Dockerfile (multi-stage), CI workflow (reusing shared 27-stage pipeline), Helm chart or Kustomize overlay, k6 load test scaffold, Flyway/Alembic migration directory.

## Rejected

## - **Single language only (Java or Python):** Limits tenant adoption. Many data teams use Python, application teams use Java. Supporting both from Phase 1 maximizes platform utility.
- **All languages from Phase 1:** Each language requires dedicated template, build adapter, and CI testing. Spreading too thin. Phase 1 focuses on the two most common (Java + Python), Phase 2 adds Go and Node.js.
- **Language-specific CI pipelines:** Separate pipeline per language = maintenance explosion. CAVE uses shared 27-stage pipeline with language-specific build adapter only (stage 8).

## Consequences

## **Positive:**
- Golden Path templates reduce time-to-first-deployment to < 5 minutes per language.
- Standardized CI stages across languages (only build adapter differs).
- New language support = new build adapter + new Backstage template.

**Negative:**
- Each language requires dedicated template maintenance.
- Language-specific debugging knowledge needed for CI issues.
- Phase 1 limits to Java + Python — tenants needing Go/Node must wait or use Expert Path.

## Compliance Mapping

## SOC2 CC8.1 (standardized development practices). ISO A.8.25 (secure development lifecycle per language).
