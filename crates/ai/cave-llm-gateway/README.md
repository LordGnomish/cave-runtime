# cave-llm-gateway

OpenAI-compatible LLM router and proxy. Six concrete providers,
keychain-resolved SaaS keys, capability-based routing, response cache,
exponential-backoff retry, Prometheus-format exposition, aggregate
health probe, and bridges to `cave-hermes` (MultiGateway) and
`cave-llm-tracker` (daily bench).

## Providers

| Provider | Locality | Transport |
| -------- | -------- | --------- |
| Ollama | local | native `/api/chat` |
| llama.cpp | local | OpenAI-compatible `/v1/chat/completions` |
| MLX-LM | local | OpenAI-compatible `/v1/chat/completions` |
| Anthropic | SaaS | `/v1/messages` |
| OpenAI | SaaS | `/v1/chat/completions` |
| Mistral | SaaS | `/v1/chat/completions` (OpenAI-compat) |

SaaS keys resolve in order:

1. `CAVE_LLM_<PROVIDER>_API_KEY` environment variable.
2. macOS Keychain — `security find-generic-password -s cave-llm-gateway-<provider>`.
3. `KeySource::NotFound` (the request will return a 401-shaped error).

No key is ever written by this crate.

## Charter v2

- Upstream: [BerriAI/litellm](https://github.com/BerriAI/litellm) `v1.85.1`.
- Source SHA: `f9c2a417a530a8369aeed96d259992040739c0f0`.
- Parity: `fill_ratio = 1.0` (23 mapped + 3 partial + 20 skipped of 46).
- ADR: [`docs/adr/ADR-153_LLM_Gateway_MVP.md`](../../docs/adr/ADR-153_LLM_Gateway_MVP.md).

## cavectl

```bash
cavectl llm-gateway providers      # registered providers + models
cavectl llm-gateway health         # aggregate health probe
cavectl llm-gateway capabilities   # capability-router seed catalogue
cavectl llm-gateway cost           # cost ledger snapshot
cavectl llm-gateway cache          # cache stats
cavectl llm-gateway bench          # trigger cave-llm-tracker bench run
cavectl llm-gateway routes         # registered routing strategies
cavectl llm-gateway usage          # per-consumer usage snapshot
cavectl llm-gateway limits         # rate-limit table
```
