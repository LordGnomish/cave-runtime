# ADR-052: AI Chat Interface — LibreChat

**Status:** Accepted

**Scope:** Azure, Hetzner, Runtime, Universal

**Category:** AI

**Related ADRs:** 009, 013

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE users (developers, platform engineers, tenants) need a web-based AI chat interface for interacting with LLMs. The interface must route through LiteLLM gateway (ADR-013), respect data classification (ADR-102), provide conversation persistence, and authenticate via platform identity (Keycloak/Okta).

## Candidates

## | Criteria | LibreChat | OpenWebUI | ChatGPT (direct) | Backstage AI plugin | Custom UI |
|---|---|---|---|---|---|
| Self-hosted | ✅ K8s, MIT license | ✅ MIT | ❌ SaaS | ✅ Backstage | ✅ |
| Multi-provider (via LiteLLM) | ✅ OpenAI-compatible API | ✅ | ❌ OpenAI only | ⚠️ Custom | ✅ |
| Conversation persistence | ✅ MongoDB backend | ✅ SQLite/PostgreSQL | ✅ (OpenAI-hosted) | ❌ | ✅ Custom |
| OIDC authentication | ✅ Native OIDC support | ⚠️ Basic auth | ❌ | ✅ Backstage auth | ✅ Custom |
| Plugin/tool system | ✅ Plugins, assistants, presets | ✅ Tools/functions | ✅ | ⚠️ | ✅ Custom |
| Admin panel | ✅ User management, model config | ⚠️ Basic | N/A | ❌ | ✅ Custom |
| Multi-tenant | ✅ User-scoped conversations | ⚠️ | ❌ | ⚠️ | ✅ Custom |
| Community | Large (25K+ GitHub stars) | Large (35K+ stars) | N/A | Small | N/A |

## Decision

## **LibreChat** (self-hosted, MIT license) as AI chat interface for all profiles. Routes through LiteLLM (ADR-013) — inherits classification routing, PII redaction, and token metering. MongoDB backend for conversation persistence. OIDC authentication via Keycloak (Hetzner) / Okta (Azure). Model presets configured per classification level.

## Rejected

## - **OpenWebUI:** Larger GitHub stars but weaker OIDC support — LibreChat's native OIDC is cleaner for enterprise identity integration. OpenWebUI's architecture is more tightly coupled to Ollama; LibreChat's OpenAI-compatible API works with any LiteLLM backend.
- **Direct ChatGPT access:** SaaS. No classification routing. No PII redaction. No tenant isolation. Data sent to OpenAI servers — contradicts restricted/confidential classification.
- **Backstage AI plugin:** No conversation persistence. Limited UI. Backstage is a developer portal, not a chat interface. Backstage AI self-service for scaffolding is separate from general-purpose AI chat.
- **Custom UI:** Build cost. LibreChat provides full-featured chat with plugin system, admin panel, and conversation management out of box.

## Consequences

## **Positive:**
- Full-featured AI chat interface with zero custom development.
- Inherits all LiteLLM protections (classification routing, PII redaction, token metering) transparently.
- OIDC authentication integrates with existing identity stack.
- MIT license — no restrictions. Active community (25K+ stars).
- Model presets enable classification-aware UX (users see only models allowed for their classification level).

**Negative:**
- MongoDB dependency for conversation storage (additional database to manage — not CNPG, separate stack).
- LibreChat upgrade path must be tracked (Renovate).
- Conversation data requires residency management (MongoDB must be in same region as tenant data classification requires).
- Plugin system security — untrusted plugins could bypass classification routing (mitigated: admin-only plugin management, OPA validates plugin allowlist).

## Compliance Mapping

## SOC2 CC6.1 (AI access controls — OIDC authentication, classification-aware presets). GDPR Art.25 (data protection by design — classification enforcement at UI level). GDPR Art.32 (security of processing — authenticated AI access). ISO A.5.15 (access control — OIDC integration). NIS2 Art.21 (AI system access controls).
