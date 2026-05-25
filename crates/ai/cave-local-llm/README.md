# cave-local-llm

Offline draft-generation daemon using Ollama (Qwen 2.5 Coder)

## Status

This crate is in the pre-open-source-launch phase. Core functionality is stable, but parity with the upstream Ollama API is actively tracked and may vary.

## Upstream

- [Ollama](https://github.com/ollama/ollama)

## Surface ported

- Local model inference via Ollama API
- Qwen 2.5 Coder integration
- Draft generation for code completion
- Offline operation without external dependencies
- Configurable context window size
- Streaming response support
- Error handling for model unavailability
- Timeout management for long-running requests
- JSON response parsing
- Logging integration for debugging

## Public API

- `LocalLlmClient` struct for managing connections
- `generate_draft` function for code completion
- `list_models` function for model discovery
- `Config` struct for runtime configuration
- `InferenceError` enum for error handling

## Tests

Unit tests cover basic inference flows and error cases. Integration tests require a running Ollama instance and are skipped in CI unless explicitly enabled.

## License

Apache-2.0

## See also

- [cave-runtime](../cave-runtime)
- [cave-local-llm](../cave-local-llm)
- [cave-oss-stack](../cave-oss-stack)
