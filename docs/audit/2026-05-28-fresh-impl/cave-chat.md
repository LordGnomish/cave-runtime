# cave-chat — Fresh-implementation coverage audit

- **Crate:** cave-chat (`/Users/gnomish/Code/cave-runtime-main/crates/ops/cave-chat`)
- **Upstream:** danny-avila/LibreChat — https://github.com/danny-avila/LibreChat
- **Upstream tag/SHA:** v0.7.6 / `9b118d42de3f99e9c8c85d0beb46d4ae5fd74a4c`
- **Upstream license:** MIT (line-port compatible with AGPL-3.0-or-later)
- **Port policy:** line-port
- **Audit date:** 2026-05-29

## CRITICAL FINDING — domain mismatch

LibreChat is an **LLM chat web application**: conversations with AI across many providers
(OpenAI, Anthropic, Google, Bedrock, Ollama, custom), agents/assistants, tools/plugins,
prompts, presets, files + RAG, token accounting, multi-strategy auth, sharing, search.

The Cave crate `cave-chat` instead implements a **Slack-style team-chat** primitive
(channels/rooms, human-to-human messages, emoji reactions, threads, presence, message
search). It shares the word "chat" and nothing else. It declares `//! Compatible with:
LibreChat` but contains no LLM call, no provider, no conversation/agent/prompt/auth/file
logic whatsoever.

Additionally:
- `lib.rs` declares only `engine`, `models`, `routes`. **`store.rs` is NOT declared as a
  module** — it is orphan/dead code, not compiled into the crate. `store.rs` also
  references types (`Channel`, `ChatStats`, `UserPresence`, `PresenceStatus`) and a
  `channel_id`/`users` shape that do not exist in `models.rs` (which uses `ChatRoom`,
  `room_id`, `user_ids`), so it would not even compile as written.
- `routes.rs` exposes a single `/api/chat/health` endpoint returning a static JSON stub.
- `engine.rs` is the only file with real logic: a handful of pure helper fns over the
  team-chat `Message`/`Reaction` model (membership check, count, filter-by-author,
  add-reaction, total-reactions).

Effective real surface against LibreChat = essentially **0%**. The matrix below scores
every LibreChat functional module against what the crate actually provides.

## Coverage matrix

| Upstream module | Capability | Cave module | Status | Notes |
|---|---|---|---|---|
| `api/app/clients/*Client.js` (OpenAI/Anthropic/Google/Ollama/ChatGPT) | LLM provider clients — send chat completion, stream tokens | — | MISSING | No LLM client of any kind; crate has no HTTP-to-provider code |
| `api/app/clients/BaseClient.js` | Conversation orchestration: build payload, context window, title gen, token budget | — | MISSING | No conversation engine; `engine.rs` is team-chat helpers |
| `api/server/controllers/AskController.js` + `routes/ask/*` | `/ask` endpoint: prompt → AI response (SSE stream) | `routes.rs` | MISSING | Only a static `/api/chat/health` route exists |
| `api/server/controllers/EditController.js` + `routes/edit` | Edit-and-resubmit a prior AI message | — | MISSING | No edit/resubmit flow |
| `api/models/Conversation.js` | Conversation persistence (CRUD, per-user, endpoint metadata) | `models.rs::ChatRoom` | MISSING | `ChatRoom` is a team channel, not an AI conversation; no endpoint/model fields, no persistence |
| `api/models/Message.js` | AI message tree (parentMessageId, sender, tokenCount, files, plugins) | `models.rs::Message` | PARTIAL | A `Message` struct exists but models human chat (author_id/content/reactions); no parent/branch tree, no token count, no sender role, no model link |
| `api/server/routes/convos.js` | Conversation list/get/update/delete/import/fork | — | MISSING | No conversation routes |
| `api/server/routes/messages.js` | Message CRUD + feedback endpoints | — | MISSING | No message routes wired; `store`/`engine` not exposed over HTTP |
| `api/server/services/Endpoints/*` | Endpoint config & option-building per provider | — | MISSING | No endpoint abstraction |
| `api/server/routes/endpoints.js` + `EndpointController.js` | List available endpoints/models to client | — | MISSING | Absent |
| `api/server/services/ModelService.js` + `routes/models.js` | Fetch & cache model lists per provider | `models.rs` | MISSING | `models.rs` is data structs, unrelated to LLM model listing |
| `api/app/clients/agents/*` + `server/controllers/agents` | Agents (custom agent loop, function calling) | — | MISSING | No agent loop |
| `api/server/services/AssistantService.js` + `controllers/assistants` | OpenAI Assistants API integration (threads/runs) | — | MISSING | Absent |
| `api/app/clients/tools/*` + `server/services/ToolService.js` | Tool/plugin registry & execution (structured + dynamic) | — | MISSING | No tools |
| `api/server/services/PluginService.js` + `controllers/PluginController.js` | Plugin auth & manifest serving | — | MISSING | Absent |
| `api/server/services/MCP.js` + `packages/mcp` | MCP server connections for tools | — | MISSING | Absent |
| `api/models/Prompt.js` + `routes/prompts.js` | Prompt library: groups, versions, sharing, permissions | — | MISSING | Absent |
| `api/models/Preset.js` + `routes/presets.js` | Saved generation presets per user | — | MISSING | Absent |
| `api/server/services/Files/*` + `routes/files` | File upload/storage strategies (Local/S3/Firebase/OpenAI/VectorDB) | `models.rs::MessageType::File` | MISSING | Only an enum variant named `File`; no upload/storage/processing |
| `api/server/services/Files/VectorDB` + `images`/`Audio`/`Code` | RAG vectorization, image/audio/code-file processing | — | MISSING | No RAG, no embeddings |
| `api/strategies/*` (jwt/local/google/github/discord/ldap/openid) | Auth strategies & session login | — | MISSING | No auth at all |
| `api/server/controllers/auth` + `routes/auth.js`, `oauth.js` | Login/register/refresh/oauth flows | — | MISSING | Absent |
| `api/models/Role.js` + `routes/roles.js` | RBAC roles & permissions | — | MISSING | Absent |
| `api/models/User.js`, `userMethods.js`, `routes/user.js` | User accounts & profile | — | MISSING | No user model (only `Uuid` ids inline) |
| `api/models/Transaction.js`, `tx.js`, `spendTokens.js`, `Balance.js` | Token accounting, cost ledger, balance/credits | `models.rs` | MISSING | No token/cost/balance concept |
| `api/models/checkBalance.js` + `routes/balance.js` | Per-request balance enforcement | — | MISSING | Absent |
| `api/server/routes/search.js` + Message search index | Full-text conversation/message search (Meilisearch) | `store.rs::search_messages` (orphan) | PARTIAL | A naive substring search exists but in dead, non-compiled `store.rs`; no index, not over AI conversations |
| `api/models/Share.js` + `routes/share.js` | Shared/public conversation links | — | MISSING | Absent |
| `api/models/ConversationTag.js` + `routes/tags.js` | Conversation tagging/bookmarks | — | MISSING | Absent |
| `api/server/services/Tokenizer.js` + `routes/tokenizer.js` | Token counting (tiktoken) for context budgeting | — | MISSING | Absent |
| `api/cache/banViolation.js`, `logViolation.js` | Abuse/rate-limit violation tracking & bans | — | MISSING | Absent |
| `api/server/services/Config/*` + `routes/config.js` | App config / `librechat.yaml` loading & client config | — | MISSING | Absent; `State` is an empty struct |
| `api/models/Banner.js` + `routes/banner.js` | Admin banner messaging | — | MISSING | Absent |
| `packages/data-provider` | Typed API client / schemas shared FE-BE | — | MISSING | Absent |
| (team-chat) reactions on messages | Emoji reactions add/dedup/count | `engine.rs` | COVERED | Real logic, but this is a Cave-original team-chat feature with **no LibreChat counterpart** (LibreChat has message *feedback*, not emoji reactions) |
| (team-chat) channels/presence/threads | Slack-style rooms, presence, threads | `store.rs` (orphan) | PARTIAL | Logic exists but in non-compiled `store.rs`; not a LibreChat capability |

### Tally (scored against LibreChat's 34 functional modules)
- COVERED: 1 (and that one is a non-LibreChat team-chat feature)
- PARTIAL: 3 (`Message` struct shape; orphan `store.rs` search; orphan channels/presence)
- MISSING: 30

## Actionable gaps for strict-TDD

Ordered lowest-effort-highest-value first. Because the crate is a wrong-domain stub, the
highest-value early work is establishing the LLM-chat skeleton (conversation model →
endpoints list → a single provider ask path) so subsequent ports have a spine.

1. **Conversation model & CRUD** — upstream `api/models/Conversation.js`, `api/server/routes/convos.js`.
   - Test `conversation_roundtrip_persists_endpoint_and_model`: create a `Conversation { conversation_id, user_id, endpoint, model, title }`, store it, fetch by id, assert all fields survive and `user_id` scoping prevents cross-user fetch.

2. **AI Message tree (parentMessageId branching)** — upstream `api/models/Message.js`.
   - Test `message_tree_resolves_parent_chain`: insert root + two children sharing a `parent_message_id`, then a grandchild; assert `ancestors(grandchild)` returns the ordered chain root→child→grandchild and that branching siblings are both retrievable.

3. **Endpoints listing** — upstream `api/server/routes/endpoints.js`, `services/Endpoints/*`.
   - Test `list_endpoints_reports_configured_providers`: configure openai+anthropic, GET `/api/endpoints`, assert JSON contains both with their model arrays and excludes unconfigured providers.

4. **Token counting / tokenizer** — upstream `api/server/services/Tokenizer.js`, `routes/tokenizer.js`.
   - Test `tokenizer_counts_known_string`: assert `count_tokens("cl100k_base", "hello world")` returns the documented tiktoken count (e.g. 2) and is deterministic.

5. **Single-provider ask path (OpenAI-compatible, streamed)** — upstream `controllers/AskController.js`, `app/clients/OpenAIClient.js`, `routes/ask/openAI.js`.
   - Test `ask_openai_streams_assistant_message`: POST `/api/ask/openAI` with a prompt against a mocked completion server, assert SSE frames are emitted and a final assistant `Message` is persisted with non-zero `token_count` and correct `parent_message_id`.

6. **Token accounting / balance** — upstream `api/models/spendTokens.js`, `Transaction.js`, `checkBalance.js`.
   - Test `spend_tokens_decrements_balance_and_records_transaction`: with balance 1000, spend prompt=10/completion=20, assert balance becomes 970 and a `Transaction` row records token counts and a derived cost.

Note: the wrong-domain team-chat code (`engine.rs`, orphan `store.rs`/`models.rs`) should
be either re-scoped out of cave-chat (it belongs in a `cave-teamchat`-style crate) or
removed, since it inflates LOC without contributing any LibreChat parity. Also fix the
build hazard: `store.rs` is declared in no module and references non-existent
`models.rs` types.
