# ADR-043: Schema Migration Validation — Flyway + Alembic

**Status:** Accepted

**Scope:** Universal

**Category:** CI/CD

**Related ADRs:** 010, 116

## Context

Database schema changes are a top cause of production incidents. CAVE's CI pipeline (stage 6) must validate that every schema migration is: (1) forward-applicable, (2) backward-rollbackable, and (3) compatible with the running application version.

## Candidates

| Criteria | Flyway (Java) + Alembic (Python) | Liquibase | Atlas | Manual SQL |
|---|---|---|---|---|
| Language support | ✅ Flyway (Java/JVM), Alembic (Python) | ✅ Multi-language | ✅ Go, any | N/A |
| Rollback validation | ✅ Undo migrations (Flyway), downgrade (Alembic) | ✅ | ✅ | ❌ |
| CI dry-run | ✅ Migrate + rollback on ephemeral DB | ✅ | ✅ | ❌ |
| Version tracking | ✅ Migration table in DB | ✅ | ✅ | ❌ |
| License | Flyway Community: Apache 2.0. Alembic: MIT | Liquibase Community: Apache 2.0 | Apache 2.0 | N/A |

## Decision

**Flyway** for Java/JVM applications. **Alembic** for Python applications. CI stage 6 runs: (1) forward migration on ephemeral DB, (2) rollback migration, (3) verify DB returns to previous state. Schema evolution blocked during cross-provider migration freeze (ADR-066).

## Rejected

- **Liquibase:** Capable but Flyway is more widely adopted for Java. Both are acceptable — Flyway chosen for simpler syntax.
- **Atlas:** Newer, Go-based. Less mature ecosystem. Would require additional tooling for Java/Python projects.
- **Manual SQL scripts:** No version tracking, no rollback validation, no CI integration. Anti-pattern.

## Consequences

**Positive:**
- Every schema change validated for forward and rollback before reaching staging.
- Ephemeral DB test — no impact on real data.
- Version tracking prevents migration conflicts across branches.

**Negative:**
- Two tools (Flyway + Alembic) for two language stacks.
- Developers must write rollback migrations for every forward migration (discipline required).
- Ephemeral DB provisioning adds ~30s to CI stage 6.

## Notes

**Universal scope** — Platform tenant DB'leri + Cave Runtime cave-pg/cave-docdb migration validation. **Runtime mirror REQUIRED**: cave-schema-migrate crate (Mirror-001 blanket scope, dual upstream Flyway+Alembic semantics tek crate'te) sovereign/disconnected deployment'larda CI-time validation kırılırsa runtime kendi migration gate'ini koşar — cave-self-improver + Reflex Engine zinciri için schema drift detection load-bearing. Atlas (Go-native + declarative HCL, 2026 olgun) gelecekte üçüncü upstream olarak değerlendirilebilir.

## Compliance Mapping

SOC2 CC8.1 (change management — schema changes validated). ISO A.14.2.9 (system acceptance testing — DB migration testing).
