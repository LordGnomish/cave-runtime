<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-144 — Code Intelligence: GitNexus

<!-- needs Burak verify: Proposed status — does GitNexus remain the OSS-launch direction? -->

> **OSS Cave Runtime context.** Originally drafted for the closed-source CAVE
> platform. In the OSS Cave Runtime, GitNexus is the reimpl target for the
> code-intelligence layer (Graph RAG over the workspace); the tool-selection
> rationale below transfers verbatim.

Status: Proposed (2026-04-26 — Burak finalize edecek)
Scope: Universal (Platform; Runtime + Pipeline inherit via ADR-RUNTIME-UPSTREAM-MIRROR-001)
Category: AI / Developer Experience / Code Intelligence
Related: ADR-011 (Backstage Portal), ADR-013 (LiteLLM Gateway), ADR-076 (cave-ctl + MCP), ADR-049 (OpenSearch), ADR-114 (Qdrant Vector DB), ADR-143 (Communication Hub), ADR-SELF-IMPROVE-001 (cave-agent)

## Context

Cave Platform'da AI agents (cave-agent self-improve loop, AI Workbench chat from ADR-143) ve developer'lar codebase'in pre-computed structural context'sine ihtiyaç duyar. Mevcut araçlar:

- **ad-hoc grep + LLM context window** — context-free, eksik dependency, hallucination riski
- **Backstage TechDocs (ADR-011)** — wiki only, no graph + no AI agent context
- **OpenSearch (ADR-049)** — full-text only, no AST graph
- **Qdrant (ADR-114)** — vector only, no structural intelligence
- **LSP servers** — single-language, no cross-language graph

**Boşluk:** Pre-computed code knowledge graph + Graph RAG pattern. AI agent'a "complete answer, 1 query" sağlar — Cursor/Claude Code/cave-agent tarzı tool'ların architectural awareness'ı için kritik.

## Decision

**GitNexus** (https://github.com/abhigyanpatwari/GitNexus, **PolyForm Noncommercial** license) — client-side code intelligence + Graph RAG engine.

### Features
- Tree-sitter AST parsing (14+ language)
- LadybugDB embedded knowledge graph
- 16+ MCP tools (impact_analysis, find_callers, find_definitions, multi_file_rename, dependency_graph, process_grouped_search)
- Hybrid search: BM25 (lexical) + semantic vector + Reciprocal Rank Fusion
- Pre-computed structural intelligence (community clustering, call-chain tracing, confidence-scored relationships)
- Browser web UI + CLI + MCP server
- Git-diff impact detection
- Wiki/docstring auto-generation

### Cave Platform deployment

- **Per-tenant GitNexus instance** (Platform-managed Helm chart)
- **MCP server endpoint** cave-agent (ADR-SELF-IMPROVE-001) ve AI Workbench (ADR-143) için
- **Backstage entity model'ine plug** (ADR-011) — code repos + dependency graph Backstage catalog'da görünür
- **Cavectl integration:** `cavectl code-intel query`, `cavectl code-intel reindex --tenant <t>`

## Reddedilen Alternatifler

- **Sourcegraph** — commercial enterprise, expensive, vendor lock-in
- **Backstage TechDocs alone** — wiki only, no graph + no AI agent context
- **GitHub Copilot Workspace** — GitHub-locked, sovereign violation
- **ad-hoc grep + LLM ad-hoc context** — hallucination + missing dependency riski
- **Custom reimpl Platform v0.1'de** — yıllar sürer, Cave Runtime kendi reimpl'i v0.2'de (mirror principle altında, Apache 2.0)

## License Consideration

**KRİTİK:** GitNexus **PolyForm Noncommercial** license. Cave Platform commercial sovereign Cloud OS olarak kullanmak için **commercial license purchase** gerekir GitNexus maintainer'ından.

**Risk:** Commercial license fiyatlandırması belirsiz veya redistribute kuralları sınırlayıcı olabilir.

**Mitigation:**
- Tenant başına yıllık license cost predictable (Cave SaaS tenant pricing'e geçirilir)
- Cave Runtime mirror v0.2'de Apache 2.0 sovereign reimpl `cave-codex` (license sınırlamasından kurtuluş)
- Sourcegraph fallback eğer GitNexus license unworkable

## Pipeline + Runtime Inheritance

- **Pipeline-platform-starter** — ADR-144 inherits eder; GitNexus Pipeline'a Helm deploy edilir
- **Runtime** — ADR-RUNTIME-UPSTREAM-MIRROR-001 charter principle altında **`cave-codex` sovereign Rust reimpl** otomatik. Yeni Runtime override ADR'ı YAZILMAZ — mirror principle yeterli. Apache 2.0 license, no PolyForm dependency.

`cave-codex` reimpl source bağımlılığı:
- Tree-sitter (Rust crate, MIT)
- petgraph (MIT)
- tantivy (MIT) — full-text BM25
- cave-iceberg (mevcut, Apache 2.0) — vector embeddings
- MCP server endpoint (Anthropic protocol)

## Implementation Phases

**v0.1 Platform (this OSS launch):** GitNexus Helm chart per-tenant deploy. cave-agent MCP entegrasyonu. Backstage entity sync. License purchase (commercial).

**v0.2 Runtime mirror:** `cave-codex` sovereign Rust reimpl Apache 2.0. Tree-sitter + petgraph + tantivy + cave-iceberg vector. Multi-tenant first-class. PQC-ready (charter binding). License sınırından kurtuluş.

**v0.3:** LSP server export + IDE integration + auto-wiki generation.

## Consequences

### Positive
- AI agents architectural awareness (cave-agent self-improve quality dramatically improves)
- Developer DX (impact analysis + multi-file rename + dependency graph viz)
- Backstage entity catalog enriched
- Cross-language graph (14+ language unified)

### Negative
- Commercial license cost (GitNexus PolyForm Noncommercial)
- Per-tenant deploy overhead (resource cost)
- Indexing time (large repos: 30-60dk full reindex)
- Storage (knowledge graph + embeddings: ~5-10x source code size)

### Risks
- **GitNexus license cost** unworkable → Sourcegraph fallback or accelerate `cave-codex` reimpl to v0.1
- **GitNexus project archive** (small open source maintainer) → fork or migrate
- **Tenant cross-contamination** → strict per-tenant Helm chart isolation + RBAC

## Compliance

- License: PolyForm Noncommercial Platform v0.1 (commercial license required) → Apache 2.0 v0.2 mirror
- SOC2 CC8.1 (change management code review aid)
- ISO 27001 A.12.6 (technical vulnerability management — impact analysis aids)
- GDPR Art.32 (security of processing — code intelligence aids vulnerability detection)
- NIS2 Art.21 (security measures)
