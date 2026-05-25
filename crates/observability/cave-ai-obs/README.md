# cave-ai-obs

LLM observability and tracing for the cave-runtime ecosystem, designed for seamless integration with Langfuse.

## Status

This crate is currently in the pre-open-source-launch phase. Feature parity with the upstream Langfuse specification is actively tracked and implemented incrementally.

## Upstream

- [Langfuse](https://langfuse.com)

## Surface ported

- Automatic context propagation for LLM requests within the cave-runtime worker threads.
- Structured logging of prompt and completion tokens with high-performance serialization.
- Trace ID generation compatible with OpenTelemetry standards for cross-service correlation.
- Batched export of telemetry data to minimize network overhead and preserve performance.
- Support for custom metadata injection to tag traces with user or session identifiers.
- Integration with the cave-runtime logging subsystem for unified log aggregation.
- Configurable sampling rates to control telemetry volume in high-throughput scenarios.
- Error handling that captures and reports LLM API failures without crashing the host.
- Memory-efficient buffer management to prevent OOM issues during peak traffic.
- Async-ready design leveraging the tokio runtime for non-blocking I/O operations.

## Public API

- `pub fn init_observation_layer(config: &ObsConfig) -> Result<()>`
- `pub struct TraceContext`
- `pub fn record_llm_request(request: LlmRequest) -> TraceId`
- `pub fn record_llm_response(response: LlmResponse) -> Result<()>`
- `pub struct ObsConfig`

## Tests

Unit tests cover context propagation and serialization correctness. Integration tests verify data export compatibility with the Langfuse ingestion API.

## License

Apache-2.0

## See also

- [../cave-runtime](../cave-runtime)
- [../cave-ai-core](../cave-ai-core)
- [../cave-logging](../cave-logging)
